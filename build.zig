const std = @import("std");

pub fn build(b: *std.Build) void {
    const test_step = b.step("test", "Run the NoeSQL native Noqeri/C++/Rust/Zig validation suite");

    const zig_tests = b.addSystemCommand(&.{ "zig", "build", "--build-file", "Native/zig/build.zig", "test" });
    test_step.dependOn(&zig_tests.step);

    const rust_tests = b.addSystemCommand(&.{ "cargo", "test", "--manifest-path", "Native/rust/Cargo.toml", "--all-targets" });
    test_step.dependOn(&rust_tests.step);

    const cpp_tests = b.addSystemCommand(&.{ "make", "-C", "Native/cpp", "test" });
    test_step.dependOn(&cpp_tests.step);

    const no_js = b.addSystemCommand(&.{
        "sh", "-c",
        "set -eu; found=$(find . -type f \\( -name '*.js' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.jsx' \\) -not -path './.git/*' -print -quit); if [ -n \"$found\" ]; then echo \"JavaScript file remains: $found\" >&2; exit 1; fi",
    });
    test_step.dependOn(&no_js.step);

    const native_step = b.step("native", "Build release-safe Zig components, Rust runtime, and C++ compatibility CLI");
    const zig_release = b.addSystemCommand(&.{ "zig", "build", "--build-file", "Native/zig/build.zig", "-Doptimize=ReleaseSafe" });
    native_step.dependOn(&zig_release.step);
    const rust_release = b.addSystemCommand(&.{ "cargo", "build", "--release", "--manifest-path", "Native/rust/Cargo.toml" });
    native_step.dependOn(&rust_release.step);
    const cpp_cli = b.addSystemCommand(&.{ "make", "-C", "Native/cpp", "native-cli" });
    native_step.dependOn(&cpp_cli.step);
}
