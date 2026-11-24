// The agent hooks, tested at their one seam: the tool call an agent is about to make in, and the
// reason it is refused out (or nothing, when it may go ahead).
import { expect, test } from "bun:test";
import { refusal, touchesManifest } from "./hook";

const DEV = { CARGO_TARGET_DIR: "target/dev" };

test("an ordinary command goes ahead", () => {
  for (const command of [
    "git status",
    "git push -u origin v11-12-docs",
    "mise run full",
    "CARGO_TARGET_DIR=target/dev nice -n 15 ionice -c3 cargo test -p svanbot-core -j4",
    "cargo fmt --all",
    "cargo --version",
    "systemctl --user status svanbot@SurSvan",
    "journalctl --user -u svanbot@SurSvan -f",
    "grep -rn 'git push origin main' docs",
  ]) {
    expect(refusal(command, {})).toBeNull();
  }
});

test("a push to main is refused, however it is spelled", () => {
  for (const command of [
    "git push origin main",
    "git push origin HEAD:main",
    "git push origin v11:main",
    "git push --force-with-lease origin +v11:refs/heads/main",
    "cd /repo && git push origin main",
  ]) {
    expect(refusal(command, {})).toContain("merged pull request");
  }
});

test("a forced push is refused on any branch", () => {
  for (const command of [
    "git push --force origin v11-12-docs",
    "git push -f",
    "git push --force-with-lease origin v11-12-docs",
    "git push origin +v11-12-docs",
  ]) {
    expect(refusal(command, {})).toContain("force");
  }
});

test("a build outside target/dev is refused, and one inside it goes ahead", () => {
  for (const command of [
    "cargo build",
    "cargo test -p svanbot-core",
    "nice cargo clippy --workspace",
  ]) {
    expect(refusal(command, {})).toContain("CARGO_TARGET_DIR=target/dev");
    expect(refusal(command, DEV)).toBeNull();
  }
  expect(refusal("CARGO_TARGET_DIR=/tmp/elsewhere cargo build", DEV)).toContain(
    "CARGO_TARGET_DIR=target/dev",
  );
});

test("a release build is refused: CI builds what the fleet runs", () => {
  expect(refusal("cargo build --release", DEV)).toContain("CI builds");
  expect(refusal("CARGO_TARGET_DIR=target/dev cargo build -r -p svanbot", {})).toContain(
    "CI builds",
  );
});

test("starting a bot is the operator's move", () => {
  for (const command of [
    "systemctl --user start svanbot@SurSvan",
    "systemctl --user enable --now svanbot@SurSvan",
    "systemctl --user restart 'svanbot@*'",
    "SVANBOT_NAME=SurSvan svanbot-release update --fleet",
    "svanbot-release rollback",
    "./scripts/start.sh",
    "bash scripts/stop.sh",
    "bash '/home/operator/checkout/scripts/start.sh'",
    "cd scripts && ./stop.sh",
  ]) {
    expect(refusal(command, DEV)).toContain("operator");
  }
  expect(refusal("svanbot-release status", DEV)).toBeNull();
  expect(refusal("bash -n scripts/start.sh", DEV)).toBeNull();
  expect(refusal("systemctl --user stop svanbot@SurSvan", DEV)).toContain("operator");
});

test("an edit of a crate manifest is one the layering check follows", () => {
  expect(touchesManifest("/repo/crates/core/Cargo.toml")).toBe(true);
  expect(touchesManifest("Cargo.toml")).toBe(true);
  expect(touchesManifest("/repo/crates/core/src/lib.rs")).toBe(false);
  expect(touchesManifest("/repo/docs/Cargo.toml.md")).toBe(false);
});
