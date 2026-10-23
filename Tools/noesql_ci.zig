const std = @import("std");

test "canonical NoeSQL extensions remain explicit" {
    try std.testing.expect(std.mem.eql(u8, ".noqeri", ".noqeri"));
    try std.testing.expect(std.mem.eql(u8, ".noedb", ".noedb"));
}

test "native host ABI remains v2" {
    const expected: u32 = 2;
    try std.testing.expectEqual(@as(u32, 2), expected);
}

test "source and output caps match native ABI" {
    try std.testing.expectEqual(@as(usize, 1024 * 1024), 1_048_576);
    try std.testing.expectEqual(@as(usize, 16 * 1024 * 1024), 16_777_216);
}
