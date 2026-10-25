const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const test_step = b.step("test", "Run the NoeSQL Rust/Zig validation suite and source-stack gates");

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

    const no_js = b.addSystemCommand(&.{
        "sh", "-c",
        "set -eu; found=$(find . -path './.git' -prune -o -path './.ci-noqeri' -prune -o -path './.ci-noqeri-build' -prune -o -type f \\( -name '*.js' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.jsx' \\) -print -quit); if [ -n \"$found\" ]; then echo \"JavaScript file remains: $found\" >&2; exit 1; fi",
    });
    test_step.dependOn(&no_js.step);

    const no_c_family = b.addSystemCommand(&.{
        "sh", "-c",
        "set -eu; found=$(find . -path './.git' -prune -o -path './.ci-noqeri' -prune -o -path './.ci-noqeri-build' -prune -o -type f \\( -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.cxx' -o -name '*.hpp' -o -name '*.hxx' \\) -print -quit); if [ -n \"$found\" ]; then echo \"C/C++ implementation source remains in NoeSQL: $found\" >&2; exit 1; fi; test ! -d Native/cpp",
    });
    test_step.dependOn(&no_c_family.step);

    const native_step = b.step("native", "Build release-safe Zig storage and Rust runtime");
    const zig_release = b.addSystemCommand(&.{ "zig", "build", "--build-file", "Native/zig/build.zig", "-Doptimize=ReleaseSafe" });
    native_step.dependOn(&zig_release.step);

    const rust_release = b.addSystemCommand(&.{
        "sh", "-c",
        "NOESQL_ZIG_LIB_DIR=\"$PWD/Native/zig/zig-out/lib\" cargo build --release --manifest-path Native/rust/Cargo.toml --features zig-pager",
    });
    rust_release.step.dependOn(&zig_release.step);
    native_step.dependOn(&rust_release.step);

    const chain_step = b.step("native-chain", "Build and execute Rust -> Noqeri AOT semantics -> Zig storage");
    const chain = b.addSystemCommand(&.{ "make", "native-chain" });
    chain_step.dependOn(&chain.step);

    const stage2_step = b.step("stage2-chain", "Build verified Stage-2 NQRI and execute Rust -> Stage-2 semantics -> Zig storage");
    const stage2 = b.addSystemCommand(&.{ "make", "stage2-chain" });
    stage2_step.dependOn(&stage2.step);
}
