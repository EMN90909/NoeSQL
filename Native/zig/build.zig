const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const bridge = b.createModule(.{
        .root_source_file = b.path("src/bridge.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const library = b.addLibrary(.{
        .name = "noesql_zig_bridge",
        .linkage = .static,
        .root_module = bridge,
    });
    b.installArtifact(library);

    const pages_library_module = b.createModule(.{
        .root_source_file = b.path("src/storage_abi.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const pages_library = b.addLibrary(.{
        .name = "noesql_zig_pages",
        .linkage = .static,
        .root_module = pages_library_module,
    });
    b.installArtifact(pages_library);

    const tests = b.addTest(.{ .root_module = bridge });
    const step = b.step("test", "Exercise the versioned Zig ABI and storage contracts");
    step.dependOn(&b.addRunArtifact(tests).step);

    const pages = b.createModule(.{
        .root_source_file = b.path("src/page_verify.zig"),
        .target = target,
        .optimize = optimize,
    });
    const page_tests = b.addTest(.{ .root_module = pages });
    step.dependOn(&b.addRunArtifact(page_tests).step);

    const wal = b.createModule(.{
        .root_source_file = b.path("src/wal_verify.zig"),
        .target = target,
        .optimize = optimize,
    });
    const wal_tests = b.addTest(.{ .root_module = wal });
    step.dependOn(&b.addRunArtifact(wal_tests).step);

    const v2_pager = b.createModule(.{
        .root_source_file = b.path("src/pager_v2.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const pager_tests = b.addTest(.{ .root_module = v2_pager });
    step.dependOn(&b.addRunArtifact(pager_tests).step);

    // Experimental v3 page allocator/manifest has independent tests;
    // not wired into the NQDB0002 storage ABI or migration path yet.
    const v3_allocator = b.createModule(.{
        .root_source_file = b.path("src/page_alloc_v3.zig"),
        .target = target,
        .optimize = optimize,
    });
    const v3_tests = b.addTest(.{ .root_module = v3_allocator });
    step.dependOn(&b.addRunArtifact(v3_tests).step);

    const v2_index = b.createModule(.{
        .root_source_file = b.path("src/index_v2.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const index_tests = b.addTest(.{ .root_module = v2_index });
    step.dependOn(&b.addRunArtifact(index_tests).step);

    const v2_abi = b.createModule(.{
        .root_source_file = b.path("src/storage_abi.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const abi_tests = b.addTest(.{ .root_module = v2_abi });
    step.dependOn(&b.addRunArtifact(abi_tests).step);
}
