# The decision path stays in-process

Every decision is made inside the bot process that holds the seat: no worker process, no remote
service, no model on the network, and no LLM on the path. The action clock is the one deadline that
matters. Every hop out of process adds latency and failure modes, and when SvanBot measured it in v10
it found nothing worth buying: 0 decision timeouts in 63,510 live decisions (2026-10-03). v11 starts
from scratch but carries this decision unchanged. A split is reconsidered only when measured decision
overruns, or a near-clock tail, show that the in-process path is the bottleneck.

## Status

accepted (2026-10-03, v10). Restated for v11 on 2026-11-15, unchanged.
