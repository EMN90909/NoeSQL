const std = @import("std");
const pages = @import("page_verify.zig");

// Read-only compatibility verifier for the historical C++ NQWL0001 full-image
// redo journal. This is NOT a Zig writer, pager, transaction manager, or fsync.
pub const VerifyError = error{
    InvalidLength, InvalidMagic, InvalidImageSize, InvalidCommit,
    ImageDigestMismatch, FrameDigestMismatch, GenerationMismatch, InvalidPageImage,
};
pub const RecoveryImage = struct { generation: u64, image: []const u8 };
const SHA_BYTES = 32;
const HEADER_BYTES = 64;
const TRAILER_BYTES = 8 + SHA_BYTES;

fn u64le(source: []const u8, offset: usize) u64 {
    var result: u64 = 0;
    var i: usize = 0;
    while (i < 8) : (i += 1) {
        result |= @as(u64, source[offset + i]) << @as(u6, @intCast(i * 8));
    }
    return result;
}
fn hashMatches(data: []const u8, digest: []const u8) bool {
    var expected: [SHA_BYTES]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(data, &expected, .{});
    return std.mem.eql(u8, digest, expected[0..]);
}
pub fn verifyLegacyCommittedWal(wal: []const u8) VerifyError!RecoveryImage {
    if (wal.len < HEADER_BYTES + TRAILER_BYTES) return error.InvalidLength;
    if (!std.mem.eql(u8, wal[0..8], "NQWL0001")) return error.InvalidMagic;
    const raw_size = u64le(wal, 16);
    if (raw_size > pages.MAX_IMAGE_BYTES or raw_size < pages.PAGE_BYTES * 2 or
        raw_size % pages.PAGE_BYTES != 0) return error.InvalidImageSize;
    const size: usize = @intCast(raw_size);
    if (wal.len != HEADER_BYTES + size + TRAILER_BYTES) return error.InvalidLength;
    const image = wal[HEADER_BYTES .. HEADER_BYTES + size];
    if (!std.mem.eql(u8, wal[HEADER_BYTES + size .. HEADER_BYTES + size + 8], "COMMIT01")) return error.InvalidCommit;
    if (!hashMatches(image, wal[24..56])) return error.ImageDigestMismatch;
    if (!hashMatches(wal[0 .. wal.len - SHA_BYTES], wal[wal.len - SHA_BYTES ..])) return error.FrameDigestMismatch;
    _ = pages.verifyLegacyImage(image) catch return error.InvalidPageImage;
    const generation = u64le(wal, 8);
    if (generation != u64le(image, 48)) return error.GenerationMismatch;
    return .{ .generation = generation, .image = image };
}
fn putU64(target: []u8, offset: usize, value: u64) void {
    var i: usize = 0;
    while (i < 8) : (i += 1) target[offset + i] = @truncate(value >> @as(u6, @intCast(i * 8)));
}
fn putU32(target: []u8, offset: usize, value: u32) void {
    var i: usize = 0;
    while (i < 4) : (i += 1) target[offset + i] = @truncate(value >> @as(u5, @intCast(i * 8)));
}
fn writeHash(data: []const u8, target: []u8) void {
    var digest: [SHA_BYTES]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(data, &digest, .{});
    @memcpy(target, digest[0..]);
}
fn sealPage(page: []u8) void {
    @memset(page[8..40], 0);
    writeHash(page, page[8..40]);
}
test "Zig validates legacy WAL commit checksums and page generation" {
    const image_size = 2 * pages.PAGE_BYTES;
    const wal_size = HEADER_BYTES + image_size + TRAILER_BYTES;
    var wal: [wal_size]u8 = [_]u8{0} ** wal_size;
    @memcpy(wal[0..8], "NQWL0001");
    putU64(wal[0..], 8, 7);
    putU64(wal[0..], 16, image_size);
    const image = wal[HEADER_BYTES .. HEADER_BYTES + image_size];
    @memcpy(image[0..8], "NQPG0001");
    putU32(image, 40, 1);
    putU32(image, 44, pages.PAGE_BYTES);
    putU64(image, 48, 7);
    putU32(image, 56, 1);
    sealPage(image[0..pages.PAGE_BYTES]);
    sealPage(image[pages.PAGE_BYTES..]);
    writeHash(image, wal[24..56]);
    @memcpy(wal[HEADER_BYTES + image_size .. HEADER_BYTES + image_size + 8], "COMMIT01");
    writeHash(wal[0 .. wal.len - SHA_BYTES], wal[wal.len - SHA_BYTES ..]);
    const verified = try verifyLegacyCommittedWal(&wal);
    try std.testing.expectEqual(@as(u64, 7), verified.generation);
    wal[HEADER_BYTES + 80] ^= 1;
    try std.testing.expectError(error.ImageDigestMismatch, verifyLegacyCommittedWal(&wal));
}
test "Zig legacy WAL verifier rejects empty, uncommitted or incompatible bytes" {
    try std.testing.expectError(error.InvalidLength, verifyLegacyCommittedWal(""));
    var bad: [104]u8 = [_]u8{0} ** 104;
    try std.testing.expectError(error.InvalidMagic, verifyLegacyCommittedWal(&bad));
    @memcpy(bad[0..8], "NQWL0001");
    try std.testing.expectError(error.InvalidImageSize, verifyLegacyCommittedWal(&bad));
}
