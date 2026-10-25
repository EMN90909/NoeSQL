const std = @import("std");
const builtin = @import("builtin");

pub const COMPONENT_VERSION = "1.0.0"; // component API, not production qualification
// Experimental Linux-only WAL pager. No SQL semantics or C++ compatibility.
// v2 is deliberately incompatible with legacy NQPG0001 and JSON .nqdb files.
// The directory containing the database MUST be owned by the application and
// inaccessible for writes by other users. Never open untrusted parent paths.
const c = struct {
    extern "c" fn open([*:0]const u8, c_int, c_uint) c_int;
    extern "c" fn close(c_int) c_int;
    extern "c" fn read(c_int, [*]u8, usize) isize;
    extern "c" fn write(c_int, [*]const u8, usize) isize;
    extern "c" fn lseek(c_int, i64, c_int) i64;
    extern "c" fn fsync(c_int) c_int;
    extern "c" fn ftruncate(c_int, i64) c_int;
    extern "c" fn flock(c_int, c_int) c_int;
    extern "c" fn rename([*:0]const u8, [*:0]const u8) c_int;
    extern "c" fn unlink([*:0]const u8) c_int;
    extern "c" fn getpid() c_int;
};
const O_RDONLY: c_int = 0;
const O_RDWR: c_int = 2;
const O_CREAT: c_int = 0x40;
const O_EXCL: c_int = 0x80;
const O_CLOEXEC: c_int = 0x80000;
const O_NOFOLLOW: c_int = 0x20000;
const O_DIRECTORY: c_int = 0x10000;
const SEEK_SET: c_int = 0;
const SEEK_END: c_int = 2;
const LOCK_EX: c_int = 2;
const LOCK_NB: c_int = 4;

pub const PAGE_BYTES: usize = 4096;
pub const MAX_PAGES: usize = 65536;
pub const MAX_TX_PAGES: usize = 1024;
pub const MAX_WAL_BYTES: usize = 64 * 1024 * 1024;
const FRAME: usize = 4 + PAGE_BYTES;
const TX_HEADER: usize = 24;
const TX_TRAILER: usize = 40;
const DB_HEADER: usize = 96;
const DB_FRAME: usize = PAGE_BYTES + 32;
const DB_MAGIC = "NQDB0002";
const TX_MAGIC = "NQTX0002";
const TX_COMMIT = "NQCM0002";
const Page = [PAGE_BYTES]u8;
const Mutation = struct { id: u32, bytes: Page };
pub const Error = error{
    UnsupportedPlatform, InvalidPath, Io, Locked, InvalidHeader,
    IncompatibleFormat, CorruptPage, CorruptWal, WalLimit, TooManyPages,
    TooManyMutations, DuplicatePage, EmptyTransaction, OutOfMemory, FaultInjected,
};

fn u32put(dst: []u8, v: u32) void {
    for (0..4) |i| dst[i] = @truncate(v >> @as(u5, @intCast(8 * i)));
}
fn u64put(dst: []u8, v: u64) void {
    for (0..8) |i| dst[i] = @truncate(v >> @as(u6, @intCast(8 * i)));
}
fn u32get(src: []const u8) u32 {
    var value: u32 = 0;
    for (0..4) |i| value |= @as(u32, src[i]) << @as(u5, @intCast(8 * i));
    return value;
}
fn u64get(src: []const u8) u64 {
    var value: u64 = 0;
    for (0..8) |i| value |= @as(u64, src[i]) << @as(u6, @intCast(8 * i));
    return value;
}
fn sha(bytes: []const u8) [32]u8 {
    var output: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(bytes, &output, .{});
    return output;
}
fn size(fd: c_int) Error!usize {
    const n = c.lseek(fd, 0, SEEK_END);
    if (n < 0 or n > 128 * 1024 * 1024) return error.Io;
    return @intCast(n);
}
fn seek(fd: c_int, n: usize) Error!void {
    if (c.lseek(fd, @intCast(n), SEEK_SET) < 0) return error.Io;
}
fn writeAll(fd: c_int, bytes: []const u8) Error!void {
    var offset: usize = 0;
    while (offset < bytes.len) {
        const n = c.write(fd, bytes[offset..].ptr, bytes.len - offset);
        if (n <= 0) return error.Io; // fail closed; EINTR is not retried by design
        offset += @intCast(n);
    }
}
fn readAll(fd: c_int, bytes: []u8) Error!void {
    var offset: usize = 0;
    while (offset < bytes.len) {
        const n = c.read(fd, bytes[offset..].ptr, bytes.len - offset);
        if (n <= 0) return error.Io;
        offset += @intCast(n);
    }
}
fn durable(fd: c_int) Error!void {
    if (c.fsync(fd) != 0) return error.Io;
}
fn syncParent(path: []const u8) Error!void {
    var buffer: [4096]u8 = undefined;
    const parent = std.fs.path.dirname(path) orelse ".";
    const z = std.fmt.bufPrintZ(&buffer, "{s}", .{parent}) catch return error.InvalidPath;
    const dfd = c.open(z.ptr, O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW, 0);
    if (dfd < 0) return error.Io;
    defer _ = c.close(dfd);
    try durable(dfd);
}
fn pathZ(path: []const u8, suffix: []const u8, buf: *[4096]u8) Error![:0]const u8 {
    if (!(std.mem.endsWith(u8, path, ".noedb") or std.mem.endsWith(u8, path, ".nqdb")) or path.len == 0 or
        std.mem.indexOfScalar(u8, path, 0) != null) return error.InvalidPath;
    return std.fmt.bufPrintZ(buf, "{s}{s}", .{ path, suffix }) catch error.InvalidPath;
}
fn openFile(name: [:0]const u8, create: bool) Error!c_int {
    const flags = O_RDWR | O_CLOEXEC | O_NOFOLLOW | (if (create) O_CREAT else @as(c_int, 0));
    const fd = c.open(name.ptr, flags, 0o600);
    if (fd < 0) return error.Io;
    return fd;
}

pub const Snapshot = struct {
    allocator: std.mem.Allocator,
    generation: u64,
    pages: []Page,
    active: [MAX_PAGES]bool,
    pub fn read(self: *const Snapshot, id: u32) ?[]const u8 {
        if (id >= MAX_PAGES or !self.active[id]) return null;
        return self.pages[id][0..];
    }
    pub fn deinit(self: *Snapshot) void { self.allocator.free(self.pages); }
};

pub const Pager = struct {
    allocator: std.mem.Allocator,
    db: c_int,
    wal: c_int,
    lock: c_int,
    path: []u8,
    pages: []Page,
    active: [MAX_PAGES]bool = [_]bool{false} ** MAX_PAGES,
    generation: u64 = 0,
    pending: [MAX_TX_PAGES]Mutation = undefined,
    pending_count: usize = 0,
    poisoned: bool = false,
    test_fault: FaultPoint = .none,
    pub const FaultPoint = enum { none, before_wal_write, after_wal_write, before_checkpoint_rename, after_checkpoint_rename };
    pub fn injectTestFault(self: *Pager, point: FaultPoint) void { if (builtin.is_test) self.test_fault = point; }
    fn failAt(self: *Pager, point: FaultPoint) Error!void { if (builtin.is_test and self.test_fault == point) return error.FaultInjected; }

    pub fn open(allocator: std.mem.Allocator, path: []const u8) Error!*Pager {
        if (builtin.os.tag != .linux) return error.UnsupportedPlatform;
        var a: [4096]u8 = undefined;
        var b: [4096]u8 = undefined;
        var l: [4096]u8 = undefined;
        const db_name = try pathZ(path, "", &a);
        const wal_name = try pathZ(path, ".wal", &b);
        const lock_name = try pathZ(path, ".lck", &l);
        const lock = try openFile(lock_name, true);
        errdefer _ = c.close(lock);
        if (c.flock(lock, LOCK_EX | LOCK_NB) != 0) return error.Locked;
        const db = try openFile(db_name, true);
        errdefer _ = c.close(db);
        const wal = try openFile(wal_name, true);
        errdefer _ = c.close(wal);
        const pager = allocator.create(Pager) catch return error.OutOfMemory;
        errdefer allocator.destroy(pager);
        const own_path = allocator.dupe(u8, path) catch return error.OutOfMemory;
        errdefer allocator.free(own_path);
        const pages = allocator.alloc(Page, MAX_PAGES) catch return error.OutOfMemory;
        errdefer allocator.free(pages);
        for (pages) |*p| @memset(p, 0);
        pager.* = .{ .allocator = allocator, .db = db, .wal = wal, .lock = lock, .pages = pages, .path = own_path };
        try pager.loadSnapshot();
        try pager.recover();
        try syncParent(path); // records first-time creation of data, WAL and lock
        return pager;
    }
    pub fn close(self: *Pager) void {
        _ = c.close(self.wal);
        _ = c.close(self.db);
        _ = c.close(self.lock);
        self.allocator.free(self.pages);
        self.allocator.free(self.path);
        self.allocator.destroy(self);
    }
    pub fn read(self: *const Pager, id: u32) ?[]const u8 {
        if (id >= MAX_PAGES or !self.active[id]) return null;
        return self.pages[id][0..];
    }
    // Transaction-local visibility: an index may be updated several times in a
    // single WAL transaction. Readers in the owning handle see staged bytes.
    pub fn readPending(self: *const Pager, id: u32) ?[]const u8 {
        if (id >= MAX_PAGES) return null;
        for (self.pending[0..self.pending_count]) |*mutation| {
            if (mutation.id == id) return mutation.bytes[0..];
        }
        return self.read(id);
    }
    pub fn snapshot(self: *const Pager) Error!Snapshot {
        const pages = self.allocator.dupe(Page, self.pages) catch return error.OutOfMemory;
        return .{ .allocator = self.allocator, .pages = pages, .generation = self.generation, .active = self.active };
    }
    pub fn stage(self: *Pager, id: u32, page: []const u8) Error!void {
        if (id >= MAX_PAGES or page.len != PAGE_BYTES) return error.TooManyPages;
        for (self.pending[0..self.pending_count]) |*m| {
            if (m.id == id) { @memcpy(m.bytes[0..], page); return; }
        }
        if (self.pending_count == MAX_TX_PAGES) return error.TooManyMutations;
        self.pending[self.pending_count].id = id;
        @memcpy(self.pending[self.pending_count].bytes[0..], page);
        self.pending_count += 1;
    }
    pub fn rollback(self: *Pager) void { self.pending_count = 0; }
    pub fn commit(self: *Pager) Error!void {
        if (self.poisoned) return error.Io;
        if (page.len != PAGE_BYTES) return error.InvalidHeader;
        if (self.pending_count == 0) return error.EmptyTransaction;
        const count = self.pending_count;
        const length = TX_HEADER + count * FRAME + TX_TRAILER;
        const end = try size(self.wal);
        if (end > MAX_WAL_BYTES - length or self.generation == std.math.maxInt(u64)) return error.WalLimit;
        const frame = self.allocator.alloc(u8, length) catch return error.OutOfMemory;
        defer self.allocator.free(frame);
        @memset(frame, 0);
        @memcpy(frame[0..8], TX_MAGIC);
        u64put(frame[8..16], self.generation + 1);
        u32put(frame[16..20], @intCast(count));
        for (self.pending[0..count], 0..) |m, i| {
            const offset = TX_HEADER + i * FRAME;
            u32put(frame[offset .. offset + 4], m.id);
            @memcpy(frame[offset + 4 .. offset + FRAME], m.bytes[0..]);
        }
        @memcpy(frame[length - TX_TRAILER .. length - 32], TX_COMMIT);
        const digest = sha(frame[0 .. length - 32]);
        @memcpy(frame[length - 32 ..], &digest);
        errdefer self.poisoned = true;
        try self.failAt(.before_wal_write);
        try seek(self.wal, end);
        // No in-memory changes until the whole commit is durably fsynced.
        try writeAll(self.wal, frame);
        try self.failAt(.after_wal_write);
        try durable(self.wal);
        for (self.pending[0..count]) |m| {
            self.pages[m.id] = m.bytes;
            self.active[m.id] = true;
        }
        self.generation += 1;
        self.pending_count = 0;
    }
    fn loadSnapshot(self: *Pager) Error!void {
        const n = try size(self.db);
        if (n == 0) return;
        if (n < DB_HEADER or n > DB_HEADER + MAX_PAGES * DB_FRAME) return error.IncompatibleFormat;
        var h: [DB_HEADER]u8 = undefined;
        try seek(self.db, 0);
        try readAll(self.db, &h);
        if (!std.mem.eql(u8, h[0..8], DB_MAGIC)) return error.IncompatibleFormat;
        if (u32get(h[8..12]) != PAGE_BYTES or u32get(h[12..16]) != MAX_PAGES) return error.InvalidHeader;
        const digest = sha(h[0..56]);
        if (!std.mem.eql(u8, h[56..88], &digest)) return error.InvalidHeader;
        self.generation = u64get(h[16..24]);
        for (0..MAX_PAGES) |id| {
            if ((h[24 + id / 8] & (@as(u8, 1) << @as(u3, @intCast(id % 8)))) == 0) continue;
            const offset = DB_HEADER + id * DB_FRAME;
            if (offset + DB_FRAME > n) return error.CorruptPage;
            var frame: [DB_FRAME]u8 = undefined;
            try seek(self.db, offset);
            try readAll(self.db, &frame);
            const actual = sha(frame[0..PAGE_BYTES]);
            if (!std.mem.eql(u8, frame[PAGE_BYTES..], &actual)) return error.CorruptPage;
            @memcpy(self.pages[id][0..], frame[0..PAGE_BYTES]);
            self.active[id] = true;
        }
    }
    fn recover(self: *Pager) Error!void {
        const n = try size(self.wal);
        if (n > MAX_WAL_BYTES) return error.WalLimit;
        const journal = self.allocator.alloc(u8, n) catch return error.OutOfMemory;
        defer self.allocator.free(journal);
        try seek(self.wal, 0);
        try readAll(self.wal, journal);
        var offset: usize = 0;
        var previous: u64 = 0;
        // Snapshot generation is the rollback floor for a replayed journal.
        // A crashed checkpoint may leave same-generation frames behind.
        const checkpoint_generation = self.generation;
        while (offset + TX_HEADER <= n) {
            const h = journal[offset..];
            if (!std.mem.eql(u8, h[0..8], TX_MAGIC)) return error.CorruptWal;
            const generation = u64get(h[8..16]);
            const count = u32get(h[16..20]);
            if (generation == 0 or count == 0 or count > MAX_TX_PAGES or
                u32get(h[20..24]) != 0) return error.CorruptWal;
            const length = TX_HEADER + @as(usize, count) * FRAME + TX_TRAILER;
            if (length > n - offset) break; // incomplete, uncommitted tail
            const record = journal[offset .. offset + length];
            if (!std.mem.eql(u8, record[length - 40 .. length - 32], TX_COMMIT)) return error.CorruptWal;
            const digest = sha(record[0 .. length - 32]);
            if (!std.mem.eql(u8, record[length - 32 ..], &digest)) return error.CorruptWal;
            if (previous != 0 and generation != previous + 1) return error.CorruptWal;
            previous = generation;
            { // Always validate historical frame ids, even after checkpoint.

                for (0..count) |i| {
                    const at = TX_HEADER + i * FRAME;
                    const id = u32get(record[at .. at + 4]);
                    if (id >= MAX_PAGES) return error.CorruptWal;
                    for (0..i) |j| {
                        const other = u32get(record[TX_HEADER + j * FRAME ..][0..4]);
                        if (id == other) return error.CorruptWal;
                    }
                }
            }
            // A committed record at the checkpoint generation must describe
            // exactly the same page bytes. Matching SHA-256 frames alone do not
            // establish that two same-generation histories are consistent.
            if (generation == checkpoint_generation) {
                for (0..count) |i| {
                    const at = TX_HEADER + i * FRAME;
                    const id = u32get(record[at .. at + 4]);
                    if (!self.active[id] or
                        !std.mem.eql(u8, self.pages[id][0..], record[at + 4 .. at + FRAME])) return error.CorruptWal;
                }
            }
            if (generation > self.generation) {
                if (self.generation == std.math.maxInt(u64) or generation != self.generation + 1) return error.CorruptWal;
                for (0..count) |i| {
                    const at = TX_HEADER + i * FRAME;
                    const id = u32get(record[at .. at + 4]);
                    @memcpy(self.pages[id][0..], record[at + 4 .. at + FRAME]);
                    self.active[id] = true;
                }
                self.generation = generation;
            }
            offset += length;
        }
        // Do not silently accept a complete older WAL against a newer
        // checkpoint; that can hide rollback or stale-file substitution.
        if (previous != 0 and previous < checkpoint_generation) return error.CorruptWal;
        if (offset != n) {
            // Never erase a complete foreign-version or malformed record. Only
            // remove an incomplete suffix that begins with a v2 magic prefix.
            const tail = journal[offset..];
            const prefix_len = @min(tail.len, TX_MAGIC.len);
            if (!std.mem.eql(u8, tail[0..prefix_len], TX_MAGIC[0..prefix_len])) return error.CorruptWal;
            // Strip an incomplete uncommitted suffix before appending anything.
            if (c.ftruncate(self.wal, @intCast(offset)) != 0) return error.Io;
            try durable(self.wal);
        }
    }
    pub fn checkpoint(self: *Pager) Error!void {
        if (self.poisoned) return error.Io;
        if (page.len != PAGE_BYTES) return error.InvalidHeader;
        if (self.pending_count != 0) return error.TooManyMutations;
        errdefer self.poisoned = true;
        var tmp_buf: [4096]u8 = undefined;
        const temporary = std.fmt.bufPrintZ(&tmp_buf, "{s}.checkpoint.tmp", .{self.path}) catch return error.InvalidPath;
        // This pager holds an exclusive writer flock and requires a private dir.
        // Remove a stale temp from an interrupted checkpoint before O_EXCL.
        _ = c.unlink(temporary.ptr);
        const fd = c.open(temporary.ptr, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW, 0o600);
        if (fd < 0) return error.Io;
        var replaced = false;
        defer {
            _ = c.close(fd);
            if (!replaced) _ = c.unlink(temporary.ptr);
        }
        var header: [DB_HEADER]u8 = [_]u8{0} ** DB_HEADER;
        @memcpy(header[0..8], DB_MAGIC);
        u32put(header[8..12], PAGE_BYTES);
        u32put(header[12..16], MAX_PAGES);
        u64put(header[16..24], self.generation);
        for (0..MAX_PAGES) |id| {
            if (self.active[id]) header[24 + id / 8] |= @as(u8, 1) << @as(u3, @intCast(id % 8));
        }
        const digest = sha(header[0..56]);
        @memcpy(header[56..88], &digest);
        try writeAll(fd, &header);
        for (0..MAX_PAGES) |id| {
            if (!self.active[id]) continue;
            try seek(fd, DB_HEADER + id * DB_FRAME);
            try writeAll(fd, &self.pages[id]);
            const page_digest = sha(&self.pages[id]);
            try writeAll(fd, &page_digest);
        }
        try durable(fd);
        var db_buf: [4096]u8 = undefined;
        const dbname = try pathZ(self.path, "", &db_buf);
        try self.failAt(.before_checkpoint_rename);
        if (c.rename(temporary.ptr, dbname.ptr) != 0) return error.Io;
        replaced = true;
        try self.failAt(.after_checkpoint_rename);
        try syncParent(self.path);
        // A committed checkpoint is visible before WAL truncation.
        if (c.ftruncate(self.wal, 0) != 0) return error.Io;
        try durable(self.wal);
        const new_db = try openFile(dbname, false);
        _ = c.close(self.db);
        self.db = new_db;
    }
};

fn testPath(allocator: std.mem.Allocator, leaf: []const u8) ![]u8 {
    const pid = c.getpid();
    return std.fmt.allocPrint(allocator, "/tmp/noesql-zig-v2-{d}-{s}.nqdb", .{pid, leaf});
}
test "durable page commit, reopening, rollback, and snapshot isolation" {
    const a = std.testing.allocator;
    const path = try testPath(a, "read");
    defer a.free(path);
    var wbuf: [4096]u8 = undefined;
    const wal = try pathZ(path, ".wal", &wbuf);
    var lbuf: [4096]u8 = undefined;
    const lock = try pathZ(path, ".lck", &lbuf);
    var dbuf: [4096]u8 = undefined;
    const data = try pathZ(path, "", &dbuf);
    defer { _ = c.unlink(wal.ptr); _ = c.unlink(lock.ptr); _ = c.unlink(data.ptr); }
    const pager = try Pager.open(a, path);
    var page: [PAGE_BYTES]u8 = [_]u8{0} ** PAGE_BYTES;
    page[0] = 11;
    try pager.stage(3, &page);
    try pager.commit();
    var snap = try pager.snapshot();
    defer snap.deinit();
    page[0] = 22;
    try pager.stage(3, &page);
    pager.rollback();
    try std.testing.expectEqual(@as(u8, 11), pager.read(3).?[0]);
    try pager.stage(3, &page);
    try pager.commit();
    try std.testing.expectEqual(@as(u8, 11), snap.read(3).?[0]);
    pager.close();
    const reopened = try Pager.open(a, path);
    defer reopened.close();
    try std.testing.expectEqual(@as(u8, 22), reopened.read(3).?[0]);
    try reopened.checkpoint();
    try std.testing.expectEqual(@as(usize, 0), try size(reopened.wal));
}
test "committed journal is checksummed and malformed complete records fail closed" {
    const a = std.testing.allocator;
    const path = try testPath(a, "corrupt");
    defer a.free(path);
    var walbuf: [4096]u8 = undefined;
    const wal = try pathZ(path, ".wal", &walbuf);
    var lbuf: [4096]u8 = undefined;
    const lock = try pathZ(path, ".lck", &lbuf);
    var dbuf: [4096]u8 = undefined;
    const data = try pathZ(path, "", &dbuf);
    defer { _ = c.unlink(wal.ptr); _ = c.unlink(lock.ptr); _ = c.unlink(data.ptr); }
    const p = try Pager.open(a, path);
    var page: [PAGE_BYTES]u8 = [_]u8{0} ** PAGE_BYTES;
    page[9] = 9;
    try p.stage(2, &page);
    try p.commit();
    p.close();
    const fd = try openFile(wal, false);
    try seek(fd, TX_HEADER + 4 + 9);
    const bad: [1]u8 = .{42};
    try writeAll(fd, &bad);
    try durable(fd);
    _ = c.close(fd);
    try std.testing.expectError(error.CorruptWal, Pager.open(a, path));
}

test "torn WAL suffix is discarded after a durable completed record" {
    const a = std.testing.allocator;
    const path = try testPath(a, "torn");
    defer a.free(path);
    var db: [4096]u8 = undefined;
    var wal: [4096]u8 = undefined;
    var lock: [4096]u8 = undefined;
    const db_name = try pathZ(path, "", &db);
    const wal_name = try pathZ(path, ".wal", &wal);
    const lock_name = try pathZ(path, ".lck", &lock);
    defer { _ = c.unlink(db_name.ptr); _ = c.unlink(wal_name.ptr); _ = c.unlink(lock_name.ptr); }
    const store = try Pager.open(a, path);
    var bytes: [PAGE_BYTES]u8 = [_]u8{0} ** PAGE_BYTES;
    bytes[0] = 41;
    try store.stage(7, &bytes);
    try store.commit();
    store.close();
    const fd = try openFile(wal_name, false);
    const before = try size(fd);
    try seek(fd, before);
    try writeAll(fd, TX_MAGIC[0..5]); // interrupted before full WAL header
    try durable(fd);
    _ = c.close(fd);
    const restored = try Pager.open(a, path);
    defer restored.close();
    try std.testing.expectEqual(@as(u8, 41), restored.read(7).?[0]);
    try std.testing.expectEqual(before, try size(restored.wal));
}

test "failed pre-WAL write poisons writer without exposing mutation" {
    const allocator = std.testing.allocator;
    const path = try testPath(allocator, "failed-write");
    defer allocator.free(path);
    var data_buf: [4096]u8 = undefined;
    var wal_buf: [4096]u8 = undefined;
    var lock_buf: [4096]u8 = undefined;
    const db_name = try pathZ(path, "", &data_buf);
    const wal_name = try pathZ(path, ".wal", &wal_buf);
    const lock_name = try pathZ(path, ".lck", &lock_buf);
    defer { _ = c.unlink(db_name.ptr); _ = c.unlink(wal_name.ptr); _ = c.unlink(lock_name.ptr); }
    const store = try Pager.open(allocator, path);
    var bytes = [_]u8{0} ** PAGE_BYTES;
    bytes[0] = 77;
    try store.stage(4, &bytes);
    store.injectTestFault(.before_wal_write);
    try std.testing.expectError(error.FaultInjected, store.commit());
    try std.testing.expect(store.read(4) == null);
    try std.testing.expectError(error.Io, store.commit());
    store.close();
    const reopened = try Pager.open(allocator, path);
    defer reopened.close();
    try std.testing.expect(reopened.read(4) == null);
}

test "stale complete WAL older than checkpoint fails closed" {
    const a = std.testing.allocator;
    const path = try testPath(a, "stale-wal");
    defer a.free(path);
    var dbbuf: [4096]u8 = undefined;
    var walbuf: [4096]u8 = undefined;
    var lockbuf: [4096]u8 = undefined;
    const data = try pathZ(path, "", &dbbuf);
    const wal = try pathZ(path, ".wal", &walbuf);
    const lock = try pathZ(path, ".lck", &lockbuf);
    defer { _ = c.unlink(data.ptr); _ = c.unlink(wal.ptr); _ = c.unlink(lock.ptr); }
    const pager = try Pager.open(a, path);
    var page: [PAGE_BYTES]u8 = [_]u8{0} ** PAGE_BYTES;
    page[0] = 11;
    try pager.stage(3, &page);
    try pager.commit();
    var old_record: [TX_HEADER + FRAME + TX_TRAILER]u8 = undefined;
    try seek(pager.wal, 0);
    try readAll(pager.wal, &old_record);
    try pager.checkpoint();
    page[0] = 22;
    try pager.stage(3, &page);
    try pager.commit();
    try pager.checkpoint();
    pager.close();
    const fd = try openFile(wal, false);
    if (c.ftruncate(fd, 0) != 0) return error.Io;
    try seek(fd, 0);
    try writeAll(fd, &old_record);
    try durable(fd);
    _ = c.close(fd);
    try std.testing.expectError(error.CorruptWal, Pager.open(a, path));
}

test "same generation checksum-valid but inconsistent WAL is rejected" {
    const a = std.testing.allocator;
    const path = try testPath(a, "conflicting-wal");
    defer a.free(path);
    var dbbuf: [4096]u8 = undefined;
    var walbuf: [4096]u8 = undefined;
    var lockbuf: [4096]u8 = undefined;
    const data = try pathZ(path, "", &dbbuf);
    const wal = try pathZ(path, ".wal", &walbuf);
    const lock = try pathZ(path, ".lck", &lockbuf);
    defer { _ = c.unlink(data.ptr); _ = c.unlink(wal.ptr); _ = c.unlink(lock.ptr); }
    const pager = try Pager.open(a, path);
    var page: [PAGE_BYTES]u8 = [_]u8{0} ** PAGE_BYTES;
    page[7] = 17;
    try pager.stage(3, &page);
    try pager.commit();
    var record: [TX_HEADER + FRAME + TX_TRAILER]u8 = undefined;
    try seek(pager.wal, 0);
    try readAll(pager.wal, &record);
    try pager.checkpoint();
    pager.close();
    record[TX_HEADER + 4 + 7] = 99;
    const digest = sha(record[0 .. record.len - 32]);
    @memcpy(record[record.len - 32 ..], &digest);
    const fd = try openFile(wal, false);
    if (c.ftruncate(fd, 0) != 0) return error.Io;
    try seek(fd, 0);
    try writeAll(fd, &record);
    try durable(fd);
    _ = c.close(fd);
    try std.testing.expectError(error.CorruptWal, Pager.open(a, path));
}
