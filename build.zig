const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const test_step = b.step("test", "Run the NoeSQL Noqeri/Rust/Zig validation suite");

    const contract_module = b.createModule(.{
        .root_source_file = b.path("Tools/noesql_ci.zig"),
        .target = target,
        .optimize = optimize,
    });
    const contract_tests = b.addTest(.{ .root_module = contract_module });
    test_step.dependOn(&b.addRunArtifact(contract_tests).step);

    const zig_tests = b.addSystemCommand(&.{ "zig", "build", "--build-file", "Native/zig/build.zig", "test" });
    test_step.dependOn(&zig_tests.step);

    const rust_tests = b.addSystemCommand(&.{ "cargo", "test", "--manifest-path", "Native/rust/Cargo.toml", "--all-targets" });
    test_step.dependOn(&rust_tests.step);

    const source_contract = b.addSystemCommand(&.{
        "sh", "-c",
        "set -eu; make no-js no-c-family",
    });
    test_step.dependOn(&source_contract.step);

    const native_step = b.step("native", "Build release-safe Zig storage and Rust runtime");
    const zig_release = b.addSystemCommand(&.{ "zig", "build", "--build-file", "Native/zig/build.zig", "-Doptimize=ReleaseSafe" });
    native_step.dependOn(&zig_release.step);

    const rust_release = b.addSystemCommand(&.{
        "sh", "-c",
        "NOESQL_ZIG_LIB_DIR=\"$PWD/Native/zig/zig-out/lib\" cargo build --release --manifest-path Native/rust/Cargo.toml --features zig-pager",
    });
    rust_release.step.dependOn(&zig_release.step);
    native_step.dependOn(&rust_release.step);

    const chain_step = b.step("native-chain", "Build and execute the Rust -> Noqeri AOT semantics -> Zig storage chain");
    const chain = b.addSystemCommand(&.{ "make", "native-chain" });
    chain_step.dependOn(&chain.step);

    const stage2_step = b.step("stage2-chain", "Build verified Stage-2 NQRI and execute Rust -> Stage-2 -> Zig");
    const stage2 = b.addSystemCommand(&.{ "make", "stage2-chain" });
    stage2_step.dependOn(&stage2.step);
}
