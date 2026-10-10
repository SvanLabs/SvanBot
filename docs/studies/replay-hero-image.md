# Preserve the table image in decision evidence

Generated-by: codex/gpt-6

The frozen latest-hand audit retained 179 big-decision records. Rerunning their recorded inputs
reproduced 95 candidate lists, with 84 same-action EV differences and no changed actions. The
largest candidate difference was 1,688.434514 chips. All 179 incorrectly claimed exact inputs.
The cohort hash is a4f664152b7a5ff2fcfbb33794abbc87aa23a85567ccec626e566823f9824beb.

The missing input was the hero's table image: `ModelStore.hero_seen` influences how opponents
respond to our range, but replay capture omitted it. All 179 had hero-image weight 0.25. A controlled
recapture supplied a known image and reproduced 84 capture/rerun failures; disabling only that
weight eliminated every failure. This isolates input loss without pretending today's image can
reconstruct the original historical image.

Replay version 4 captures the per-bot image and aggregate fallback, distinguishing known absence
from an unknown legacy input. The fixed capture/JSON/rerun loop reproduces all 179 recaptured
choices and candidate lists with the known image. Both per-bot and aggregate fallback have a
focused regression that failed before the repair. No policy code or live decision changes.

The original historical records remain incomplete: the repaired review reports 179 what-if inputs,
zero verified exact replays and zero changed actions. Original weights were never recorded and
cannot honestly be restored. The identified review binary SHA256 is
564e5e20223f88fbe909d781713d4eb21dd71243e18cc278aef80a0dedbfe97c; the recapture probe is
dfd3ed3e00688e3d0f5ba1f20a1fb77d4ef8198732c6d72f2cde05d4c57dab2f. All runs used a disposable
copy, the stored networks and recorded parameters; the live databases were not changed.

Exact replay counts now require complete inputs as well as equal outputs. The analyst and drift
checks skip incomplete inputs, and tactical-loss findings require version 4 or newer. The wiring
cache key and drift basis change so previously stored evidence is not silently reused under the
repaired interpretation. Older records remain available for explicitly labelled what-if review.
This repair changes evidence capture and interpretation; it makes no strategy or performance claim.
