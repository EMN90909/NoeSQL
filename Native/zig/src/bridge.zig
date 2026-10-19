// Force the v2 page exports into the native bridge library while retaining
// the versioned v1 query callback ABI. The v2 ABI is storage-only.
comptime {
    _ = @import("storage_abi.zig").noesql_zig_page_open;
    _ = @import("storage_abi.zig").noesql_zig_page_get;
    _ = @import("storage_abi.zig").noesql_zig_page_stage;
    _ = @import("storage_abi.zig").noesql_zig_page_commit;
    _ = @import("storage_abi.zig").noesql_zig_page_rollback;
    _ = @import("storage_abi.zig").noesql_zig_page_checkpoint;
    _ = @import("storage_abi.zig").noesql_zig_page_generation;
    _ = @import("storage_abi.zig").noesql_zig_page_validate_reference_state;
    _ = @import("storage_abi.zig").noesql_zig_page_close;
}
const std = @import("std");

// This Zig module replaces the historical C++ *dispatch bridge* only.
// Query parsing and execution belong to the Noqeri-compiled callback.
// Do not confuse this with the separate POSIX C++ page-store prototype.
pub const ABI_VERSION: u32 = 1;
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
pub const INVALID_ARGUMENT: i32 = -1;
pub const UNSUPPORTED_ABI: i32 = -2;
pub const LIMIT_EXCEEDED: i32 = -3;

pub const ExecuteFn = *const fn (
    source: ?[*]const u8,
    source_length: usize,
    output: ?[*]u8,
    output_capacity: usize,
    output_length: ?*usize,
) callconv(.c) i32;

pub const EngineV1 = extern struct {
    abi_version: u32,
    execute: ?ExecuteFn,
};

pub export fn noesql_zig_validate_engine(engine: ?*const EngineV1) i32 {
    const value = engine orelse return INVALID_ARGUMENT;
    if (value.abi_version != ABI_VERSION) return UNSUPPORTED_ABI;
    if (value.execute == null) return INVALID_ARGUMENT;
    return 0;
}

pub export fn noesql_zig_execute(
    engine: ?*const EngineV1,
    source: ?[*]const u8,
    source_length: usize,
    output: ?[*]u8,
    output_capacity: usize,
    output_length: ?*usize,
) i32 {
    // Reset early so callers never consume stale lengths following an error.
    const written = output_length orelse return INVALID_ARGUMENT;
    written.* = 0;
    const valid = noesql_zig_validate_engine(engine);
    if (valid != 0) return valid;
    if (source_length > MAX_SOURCE_BYTES or output_capacity > MAX_OUTPUT_BYTES) return LIMIT_EXCEEDED;
    if (source_length != 0 and source == null) return INVALID_ARGUMENT;
    if (output_capacity != 0 and output == null) return INVALID_ARGUMENT;
    const status = engine.?.execute.?(source, source_length, output, output_capacity, written);
    if (status != 0) {
        written.* = 0;
        return status;
    }
    if (written.* > output_capacity) {
        written.* = 0;
        return LIMIT_EXCEEDED;
    }
    return 0;
}

fn echo(
    source: ?[*]const u8,
    length: usize,
    output: ?[*]u8,
    capacity: usize,
    written: ?*usize,
) callconv(.c) i32 {
    if (length > capacity) return LIMIT_EXCEEDED;
    if (length != 0) @memcpy(output.?[0..length], source.?[0..length]);
    written.?.* = length;
    return 0;
}

fn oversize(_: ?[*]const u8, _: usize, _: ?[*]u8, capacity: usize, written: ?*usize) callconv(.c) i32 {
    written.?.* = capacity + 1;
    return 0;
}
fn fails(_: ?[*]const u8, _: usize, _: ?[*]u8, _: usize, written: ?*usize) callconv(.c) i32 {
    written.?.* = 99;
    return -42;
}

test "Zig C ABI validates callbacks and compatible versions" {
    const valid = EngineV1{ .abi_version = ABI_VERSION, .execute = echo };
    const invalid = EngineV1{ .abi_version = 2, .execute = echo };
    const missing = EngineV1{ .abi_version = ABI_VERSION, .execute = null };
    try std.testing.expectEqual(@as(i32, 0), noesql_zig_validate_engine(&valid));
    try std.testing.expectEqual(UNSUPPORTED_ABI, noesql_zig_validate_engine(&invalid));
    try std.testing.expectEqual(INVALID_ARGUMENT, noesql_zig_validate_engine(&missing));
    try std.testing.expectEqual(INVALID_ARGUMENT, noesql_zig_validate_engine(null));
}

test "Zig dispatcher forwards bytes without owning database semantics" {
    const engine = EngineV1{ .abi_version = ABI_VERSION, .execute = echo };
    const input: []const u8 = "Noqeri owns execution";
    var output: [64]u8 = undefined;
    var written: usize = 777;
    try std.testing.expectEqual(@as(i32, 0), noesql_zig_execute(&engine, input.ptr, input.len, output[0..].ptr, output.len, &written));
    try std.testing.expectEqual(input.len, written);
    try std.testing.expectEqualSlices(u8, input, output[0..written]);
}

test "Zig dispatcher fails closed on bounds and callback errors" {
    const valid = EngineV1{ .abi_version = ABI_VERSION, .execute = echo };
    const bad = EngineV1{ .abi_version = ABI_VERSION, .execute = oversize };
    const fail = EngineV1{ .abi_version = ABI_VERSION, .execute = fails };
    var output: [2]u8 = undefined;
    var written: usize = 10;
    try std.testing.expectEqual(INVALID_ARGUMENT, noesql_zig_execute(&valid, null, 1, output[0..].ptr, output.len, &written));
    try std.testing.expectEqual(@as(usize, 0), written);
    try std.testing.expectEqual(LIMIT_EXCEEDED, noesql_zig_execute(&valid, null, MAX_SOURCE_BYTES + 1, output[0..].ptr, output.len, &written));
    try std.testing.expectEqual(LIMIT_EXCEEDED, noesql_zig_execute(&bad, null, 0, output[0..].ptr, output.len, &written));
    try std.testing.expectEqual(@as(usize, 0), written);
    try std.testing.expectEqual(@as(i32, -42), noesql_zig_execute(&fail, null, 0, output[0..].ptr, output.len, &written));
    try std.testing.expectEqual(@as(usize, 0), written);
}
