const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const test_step = b.step("test", "Run the NoeSQL native Rust/C++/Zig validation suite");

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

    const cpp_tests = b.addSystemCommand(&.{ "make", "-C", "Native/cpp", "test" });
    test_step.dependOn(&cpp_tests.step);

    const no_js = b.addSystemCommand(&.{
        "sh", "-c",
        "set -eu; found=$(find . -path './.git' -prune -o -path './.ci-noqeri' -prune -o -path './.ci-noqeri-build' -prune -o -type f \\( -name '*.js' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.jsx' \\) -print -quit); if [ -n \"$found\" ]; then echo \"JavaScript file remains: $found\" >&2; exit 1; fi",
    });
    test_step.dependOn(&no_js.step);

    const native_step = b.step("native", "Build release-safe Zig storage, Rust runtime, and C++ compatibility CLI");
    const zig_release = b.addSystemCommand(&.{ "zig", "build", "--build-file", "Native/zig/build.zig", "-Doptimize=ReleaseSafe" });
    native_step.dependOn(&zig_release.step);

    const rust_release = b.addSystemCommand(&.{
        "sh", "-c",
        "NOESQL_ZIG_LIB_DIR=\"$PWD/Native/zig/zig-out/lib\" cargo build --release --manifest-path Native/rust/Cargo.toml --features zig-pager",
    });
    rust_release.step.dependOn(&zig_release.step);
    native_step.dependOn(&rust_release.step);

    const cpp_cli = b.addSystemCommand(&.{ "make", "-C", "Native/cpp", "native-cli" });
    native_step.dependOn(&cpp_cli.step);

    const chain_step = b.step("native-chain", "Build and execute the Rust -> Noqeri AOT -> Zig integration chain");
    const chain = b.addSystemCommand(&.{ "make", "native-chain" });
    chain_step.dependOn(&chain.step);
}
