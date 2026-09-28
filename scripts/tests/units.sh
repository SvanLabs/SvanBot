#!/usr/bin/env bash
# Tests for scripts/units.sh (#15): the systemd user units are rendered for the checkout they are
# installed from, and never carry a directory chosen on some other machine.
#
# The regression this guards is the one #15 names — units shipped with `/srv/svanbot10` in them,
# hand-edited after every install and never following a `git pull`. So the first assertion is about
# the *templates in the tree*, not about a fixture: any absolute path baked into a unit setting fails
# the suite wherever it is added.
#
# The awkward-path case renders a copy of the checkout under a directory holding a space, a `%` and
# an `&` — all legal in a Linux home directory, all of them things a naive renderer loses — and, where
# systemd is installed, asks real systemd to resolve the result. A fixture that only compared strings
# would pass on a unit systemd reads differently, which is the failure this file exists to avoid.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd -P)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() { echo "units test: $*" >&2; exit 1; }

units="$repo/scripts/units.sh"
[ -x "$units" ] || fail "$units is not executable"

# 1. No unit template names a fixed directory. The settings that take a path are the ones that broke:
#    a checkout is named as @SVANBOT_ROOT@ or it is wrong for everyone but the machine it was typed on.
for tpl in "$repo"/scripts/svanbot10*.service "$repo"/scripts/svanbot10*.timer; do
  baked=$(grep -nE '^(WorkingDirectory|EnvironmentFile|ExecStart|ExecStop|ExecReload)=-?"?/' "$tpl" || true)
  [ -z "$baked" ] || fail "$tpl names an absolute path instead of @SVANBOT_ROOT@:"$'\n'"$baked"
done

# 2. Rendered for this checkout: the placeholder is gone, every service works in the checkout, and
#    every script an Exec line names is really there and really executable.
DEST="$t/here" "$units" > "$t/here.log"
for u in "$t"/here/svanbot10*; do
  ! grep -q '@SVANBOT_ROOT@' "$u" || fail "$u still holds the placeholder"
  ! grep -q '/srv/svanbot10' "$u" || fail "$u still names the reference machine's directory"
done
for u in "$t"/here/*.service; do
  wd=$(sed -n 's/^WorkingDirectory=//p' "$u")
  [ "$wd" = "$repo" ] || fail "$u works in $wd, not in the checkout $repo"
done
# Every `scripts/...` target of an Exec line, with the quotes systemd strips taken off.
mapfile -t targets < <(sed -n 's/^Exec[A-Za-z]*=//p' "$t"/here/*.service | sed 's/^"//; s/".*$//; s/ .*$//' | grep '/scripts/')
[ "${#targets[@]}" -ge 3 ] || fail "expected the fleet, keepalive and cleanup scripts among the Exec lines"
for exe in "${targets[@]}"; do
  [ -x "$exe" ] || fail "a rendered unit runs $exe, which is not an executable file"
done

# 3. A checkout under a path with a space, a `%` and an `&`. `%` starts a systemd specifier, so it has
#    to come out doubled; `%h` in the template is ours and must not be touched; the space and the `&`
#    have to survive verbatim.
odd="$t/od d %25 r & p"
mkdir -p "$odd/target/release"
cp -r "$repo/scripts" "$odd/scripts"
printf '#!/bin/sh\n' > "$odd/target/release/archive"
chmod +x "$odd/target/release/archive"
DEST="$t/odd" "$odd/scripts/units.sh" > /dev/null
fleet="$t/odd/svanbot10.service"
[ "$(sed -n 's/^WorkingDirectory=//p' "$fleet")" = "${odd//%/%%}" ] || fail "the odd checkout path was not escaped in WorkingDirectory"
grep -qF "ExecStart=\"${odd//%/%%}/scripts/start.sh\"" "$fleet" || fail "ExecStart lost or mangled the odd checkout path"
grep -q '^Environment=PATH=%h/\.cargo/bin' "$fleet" || fail "the template's own %h specifier was escaped away"
if command -v systemd-analyze > /dev/null; then
  for u in "$t"/odd/*.service "$t"/odd/*.timer; do
    systemd-analyze verify --user "$u" > "$t/verify.log" 2>&1 || fail "systemd rejects $u:"$'\n'"$(cat "$t/verify.log")"
  done
fi

# 4. Running it again writes nothing: a relocation re-renders, and an install that did not move must
#    not replace files systemd is watching. Inodes, because `mv -f` gives a replaced file a new one.
before=$(stat -c '%n %i' "$t"/here/svanbot10* | sort)
DEST="$t/here" "$units" > "$t/again.log"
after=$(stat -c '%n %i' "$t"/here/svanbot10* | sort)
[ "$before" = "$after" ] || fail "a second render replaced files that had not changed"
grep -q 'written' "$t/again.log" && fail "a second render reported a write"
installed=("$t"/here/svanbot10*)
[ "$(grep -c 'unchanged' "$t/again.log")" -eq "${#installed[@]}" ] || fail "not every unit was reported unchanged"

# 5. --check is the same question asked of what is installed.
DEST="$t/here" "$units" --check > /dev/null || fail "--check called a fresh install drift"
echo 'Nice=0' >> "$t/here/svanbot10-clean.service"
out=$(DEST="$t/here" "$units" --check 2>&1) && fail "--check passed an edited unit"
case "$out" in *svanbot10-clean.service*differs*) ;; *) fail "--check did not name the edited unit: $out";; esac
DEST="$t/here" "$units" > /dev/null
DEST="$t/here" "$units" --check > /dev/null || fail "re-rendering did not settle the drift"
rm "$t/here/svanbot10-keepalive.timer"
out=$(DEST="$t/here" "$units" --check 2>&1) && fail "--check passed a missing unit"
case "$out" in *svanbot10-keepalive.timer*'not installed'*) ;; *) fail "--check did not name the missing unit: $out";; esac

# 6. The case the issue is about: units installed by one checkout, checked from another. Every unit
#    is drift, because every one names the checkout that wrote it.
out=$(DEST="$t/odd" "$units" --check 2>&1) && fail "--check passed units rendered for another checkout"
for name in svanbot10.service svanbot10-archive.service svanbot10-clean.service svanbot10-keepalive.service; do
  case "$out" in *"$name"*) ;; *) fail "--check did not report $name as rendered elsewhere: $out";; esac
done

# 7. --print is the single unit, and an unknown name is an error rather than an empty file.
DEST="$t/here" "$units" --print svanbot10.service | cmp -s - <(DEST="$t/print" "$units" > /dev/null; cat "$t/print/svanbot10.service") \
  || fail "--print and the installed file disagree"
"$units" --print no-such.service > /dev/null 2>&1 && fail "--print accepted a unit that does not exist"

# 8. --setup (#463): where a user systemd instance answers, the units are installed and the service
#    plus the timers are enabled; where none answers, nothing fails and the output says how to
#    enable by hand. systemctl is a fake on PATH that logs its calls.
mkdir -p "$t/fakebin"
cat > "$t/fakebin/systemctl" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$SYSLOG"
if [ "$1 $2" = "--user daemon-reload" ] && [ -n "${NO_SYSTEMD:-}" ]; then exit 1; fi
exit 0
EOF
chmod +x "$t/fakebin/systemctl"
export SYSLOG="$t/syscalls.log"
: > "$SYSLOG"
PATH="$t/fakebin:$PATH" DEST="$t/setup" "$units" --setup > "$t/setup.log" || fail "--setup failed where systemd answers"
grep -q 'enable.*svanbot10.service.*svanbot10-keepalive.timer.*svanbot10-archive.timer.*svanbot10-clean.timer' "$SYSLOG" \
  || fail "--setup did not enable the service and the timers:"$'\n'"$(cat "$SYSLOG")"
grep -qi 'enabl' "$t/setup.log" || fail "--setup did not say what it enabled"
[ -f "$t/setup/svanbot10.service" ] || fail "--setup did not install the units"
: > "$SYSLOG"
PATH="$t/fakebin:$PATH" DEST="$t/setup-skip" NO_SYSTEMD=1 "$units" --setup > "$t/setup-skip.log" \
  || fail "--setup failed where no systemd answers"
grep -q 'enable' "$SYSLOG" && fail "--setup called enable with no systemd instance"
grep -qi 'no systemd' "$t/setup-skip.log" || fail "--setup did not say it was skipping:"$'\n'"$(cat "$t/setup-skip.log")"

echo "units tests: ok"
