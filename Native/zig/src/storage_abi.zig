const std = @import("std");
const storage = @import("pager_v2.zig");

// Native Zig-owned pager ABI for FUTURE compiled Noqeri callers.
// ABI v2, experimental Linux-only; must not be treated as a public unauthenticated
// SQL, multi-tenant, or database-server interface. Caller owns authorization.
pub const COMPONENT_VERSION = "1.0.0";
pub const ABI_VERSION: u32 = 2;
pub const STATUS_OK: i32 = 0;
pub const STATUS_NOT_FOUND: i32 = 1;
pub const STATUS_INVALID: i32 = -1;
pub const STATUS_FAILURE: i32 = -2;
pub const STATUS_BUFFER_TOO_SMALL: i32 = -3;

pub export fn noesql_zig_page_open(version: u32, name: ?[*:0]const u8, out: ?*?*anyopaque) i32 {
    const dest = out orelse return STATUS_INVALID;
    dest.* = null;
    if (version != ABI_VERSION) return STATUS_INVALID;
    const path = name orelse return STATUS_INVALID;
    const value = std.mem.span(path);
    if (value.len == 0 or value.len > 4000) return STATUS_INVALID;
    const pager = storage.Pager.open(std.heap.page_allocator, value) catch return STATUS_FAILURE;
    dest.* = @ptrCast(pager);
    return STATUS_OK;
}
fn cast(handle: ?*anyopaque) ?*storage.Pager {
    const value = handle orelse return null;
    return @ptrCast(@alignCast(value));
}
pub export fn noesql_zig_page_close(handle: ?*anyopaque) void {
    if (cast(handle)) |pager| pager.close();
}
pub export fn noesql_zig_page_stage(handle: ?*anyopaque, page_id: u32, bytes: ?[*]const u8, size: usize) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    const ptr = bytes orelse return STATUS_INVALID;
    if (size != storage.PAGE_BYTES) return STATUS_INVALID;
    pager.stage(page_id, ptr[0..size]) catch return STATUS_FAILURE;
    return STATUS_OK;
}
pub export fn noesql_zig_page_commit(handle: ?*anyopaque) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    pager.commit() catch return STATUS_FAILURE;
    return STATUS_OK;
}
pub export fn noesql_zig_page_rollback(handle: ?*anyopaque) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    pager.rollback();
    return STATUS_OK;
}
pub export fn noesql_zig_page_get(handle: ?*anyopaque, page_id: u32, output: ?[*]u8, capacity: usize) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    const src = pager.read(page_id) orelse return STATUS_NOT_FOUND;
    if (capacity < storage.PAGE_BYTES) return STATUS_BUFFER_TOO_SMALL;
    const dest = output orelse return STATUS_INVALID;
    @memcpy(dest[0..storage.PAGE_BYTES], src);
    return STATUS_OK;
}
pub export fn noesql_zig_page_generation(handle: ?*anyopaque) u64 {
    const pager = cast(handle) orelse return 0;
    return pager.generation;
}
pub export fn noesql_zig_page_checkpoint(handle: ?*anyopaque) i32 {
    const pager = cast(handle) orelse return STATUS_INVALID;
    pager.checkpoint() catch return STATUS_FAILURE;
    return STATUS_OK;
}

test "native ABI rejects untrusted arguments without dereferencing page buffers" {
    var handle: ?*anyopaque = undefined;
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_open(3, null, &handle));
    try std.testing.expect(handle == null);
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_stage(null, 0, null, 0));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_commit(null));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_rollback(null));
    try std.testing.expectEqual(STATUS_INVALID, noesql_zig_page_get(null, 0, null, 0));
}
