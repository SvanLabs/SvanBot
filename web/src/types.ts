export interface Estimate { value: number | null; count?: number; samples?: number; standard_error?: number; exact?: boolean; lower?: number; upper?: number }
export interface Decision { effective_stack?: number | null; spr?: number | null; hand_category?: string | null; best_five?: string[]; opening_threshold?: number | null; preflop_score?: number | null; opening_position?: string | null; opponent_models?: OpponentModelInput[]; action: string; amount: number; reason: string; source: string; equity: Estimate | null; pot_odds?: number; version?: string; latency_ms?: number; street?: string; pot?: number; to_call?: number; candidates?: Candidate[] | null; experiment?: Record<string, unknown> }
/** One option the policy priced for the live decision (the EV search's own table; `ev` is chips). */
export interface Candidate { action: string; amount: number | null; ev: number; category?: string; bias?: number; equity_called?: number; fold_prob?: number }
/** The shrunk profile the policy priced an opponent with (rates 0..1; fold/raise vs bet are flop, turn, river). */
export interface OpponentModelInput { name: string; archetype?: string; hands: number; confidence: number; vpip: number; pfr: number; three_bet: number; fold_to_3bet: number; cbet: number; fold_to_cbet: number; fold_vs_bet: number[]; raise_vs_bet: number[]; river_bluff: number }
/** How the opponent models read a seated opponent (0212): shrunk rates and, when installed, the per-opponent correction (fold, call, raise) of the response network. */
export interface SeatRead { style: string; hands: number; vpip: number; pfr: number; three_bet: number; fold_vs_bet: number[]; response_ratio?: number[] | null; fold_offset?: number | null; size_tell?: number | null }
export interface Seat { seat: number; name: string | null; stack: number; bet?: number; folded?: boolean; status?: string; last_action?: string; avatar_url?: string | null; read?: SeatRead | null }
/** Which season a panel's figures cover. `scoped` false means the boundary was unknown and the figures are all-time. */
export interface SeasonScope { scoped: boolean; number: number | null; id: string | null; started_at: number | null }
/** Season-scoped play figures; `all_time` carries the same measures over every season. */
/** Per-bot figures. When the store read fails the server sends the last good reading with `stale:
 *  true` and `error` set — never zeros — so a bot at a table cannot read as one with 0 hands (0326). */
export interface Metrics { hands: number; priced_hands: number; net_chips: number; bb100: number | null; confidence: number | null; season: SeasonScope; all_time: { hands: number; priced_hands: number; net_chips: number; bb100: number | null; confidence: number | null }; p95_ms: number | null; rejected: number; state_hash?: {ok:number;bad:number}; decisions: number; series: {hand:number;total:number;showdown:number;other:number;ev?:number}[]; ev_net_chips?: number; ev_bb100?: number | null; ev_confidence?: number | null; luck_chips?: number; stale?: boolean; error?: string | null }
export interface Bot { slot: number; name: string; mode: string; status: string; connected: boolean; last_error: string | null; table_id: string | null; hand_id: string | null; board: string[]; hole: string[]; seats: Seat[]; hero_seat: number | null; dealer_seat: number | null; actor_seat: number | null; pot: number; big_blind: number; street: string; decision: Decision | null; version: string; metrics: Metrics; turn_started: number | null; turn: unknown; season: Record<string, number | string>; remote?: boolean; heartbeat_age_s?: number }
/** What a table view may not expect from the table payload: the operator's own material.
 *  `metrics` is not in the table payload at all, and the public TV's projection drops the rest on
 *  purpose (`crates/apps/bot/src/api/tv.rs`) — the policy's working (`decision`, `version`), the
 *  think clock (`turn`, `turn_started`), the bot's own cards (`hole`), the street, the season and
 *  the last error. Optional rather than absent-so-far, because one listener really does omit them. */
type OperatorOnly = 'metrics' | 'hole' | 'street' | 'decision' | 'version' | 'turn' | 'turn_started' | 'season' | 'last_error';

/** A bot as the table payload carries it: everything a table view needs, and only what both
 *  listeners send. The dashboard fills `hole` in and the public TV never does, so a table that
 *  wants the hero's cards has to say what it draws when they are not there. */
export type TableBot = Omit<Bot, OperatorOnly> & Partial<Pick<Bot, OperatorOnly>>;
export interface EvaluationSummary { hands:number; mean_bb?:number|null; lower_95?:number|null; upper_95?:number|null }
export interface PopulationSummary { id:string; opponent_count:number; evidence:number; evaluation?:{id:string;basis:string;sampled:number;eligible:number;excluded:number;cutoff:number} }
export interface Experiment { id: string; status: string; ts: number; hands?: number; target?: number; knob?: string; old?: number; new?: number; mean_bb?: number; lower_95?: number; upper_95?: number; champion?: string; challenger?: string; candidate_kind?:string; rationale?:string; resumed?:boolean; terminal_reason?:string; population?:PopulationSummary; strata?:Record<'synthetic'|'observed',EvaluationSummary> }
/** Why search candidates died over the last `hours` (#317): counts by outcome key, then by knob. */
export interface SearchFunnel { hours:number; total:number; outcomes:{key:string;count:number}[]; knobs:{key:string;count:number}[] }
export interface Storage { checked_at?: number; database_check?: string; digests_checked?: number; digest_mismatches?: number; last_backup?: string | null; archive?: string | null; archive_latest?: string | null; corpus?: Record<string, number>; tables?: {flop: boolean; turn: boolean} }
export interface LearningItem { what: string; updated: number | null; detail: string }
export interface AuditSummary { decisions:number; same_action:number; mean_gap_bb:number; max_gap_bb:number; mean_deep_ms:number; queued:number }
export interface Finding { id:string; title:string; severity:string; evidence:string; value:number; since:number; updated:number; ticket?:string }
/** One class of decisions in the deep re-solve's table (#321): the numbers on the row, and what the
 *  floor made of them — `filed`, `measured`, `thin` (under the verdict floor) or `never queued`. */
export interface ClassRow { id:string; label:string; n:number; days:number; total:number; mean:number; lo:number|null; hi:number|null; big:number; decisions:number|null; state:string }
export interface StaleLoop { name:string; message:string }
/** One strategy parameter, as `crates/apps/bot/src/knobs.rs` defines it (#322): the bounds the learner
 *  searches it over — which are the Champion profile's track — the shipped default the bar marks, the
 *  decimals its value is printed with, and the sentence under it. The dashboard holds no copy of this
 *  list: a knob the search gains appears here, and one it drops stops being drawn. */
export interface ChampionKnob { key:string; label:string; min:number; max:number; decimals:number; default:number; description:string }
export interface Training { promotion_count?:number; last_promotion?: (Partial<Experiment> & {timestamp_basis?:string}) | null; findings?: {at?:number; findings?:Finding[]; cleared?:{id:string;title:string;ticket?:string|null}[]; unanswered?:string[]; coverage?:string[]; classes?:ClassRow[]; legends?:Record<string,string>} | null; stale_loops?: StaleLoop[]; settings?: {cooldown_minutes:number; max_cooldown_minutes:number; min_new_hands?:number; max_min_new_hands?:number}; next_job?: {kind:'refit'|'search';label:string;hands:number;target:number;remaining:number;reason:string;season_day?:number|null}; cooldown_until?: number | null; cooldown_minutes?: number; follow_up?: boolean; audit?: {last_24h: AuditSummary; running: boolean; analyst?: {threads:number; samples:number; audited:number; skipped:number; queued:number;busy_share:number; updated:number} | null}; learning?: LearningItem[]; storage?: Storage; past_hands?: {downloaded:number; imported_through?:number|null; bots?: Record<string,{server_total:number;backfilled:boolean}>}; range_model?: {active:boolean; fitted_at:number; showdowns:number; gain_fitted?:number|null; gain_default?:number|null}; next_run?: number; failures?: number; last_error?: string | null; status: string; job?:'refit'|'search'; phase?:string; stratum?:string|null; resumed?:boolean; resume_id?:string|null; automatic: boolean; can_rollback: boolean; champion: Record<string, string | number | boolean> & {version: string; name?: string}; knobs?: ChampionKnob[]; progress: { hands: number; target: number; mean_bb?: number; challenger_decisions?: number; challenger_policy_hits?: number }; experiments: Experiment[]; search_funnel?: SearchFunnel; opponent_profiles_tracked: number; lineage?:string[]; next_candidate?:{family:string;reason:string}; leaderboard_evidence?:{strategy:string;neural:string;range:string;calibration:string;opportunity?:string|null;next_candidate?:string|null}; neural_ev_status?: string; neural_ev?: {status?: string; fallback?: string; validation_hands?: number; validation_bb100?: number | null; required_improvement_bb100?: number} }
export interface Log { id:number; slot:number | null; ts:number; level:string; message:string }
/** Operations outside the tables: hands awaiting a store retry, the keepalive hold and its last action. */
export interface Ops { unstored_hands: number; hold_until: number | 'forever' | null; keepalive_last: string | null }
export interface Snapshot { bots: Bot[]; training: Training; logs: Log[]; updated: number; ops?: Ops; config: {buy_in:number;auto_rebuy:boolean;host:string;port:number;configured_slots:number} }
/** The dashboard board's arrangement (`GET`/`POST /api/layout`, #729): which widgets the board
 *  shows, their column and order. Stored on the server so an arrangement survives a browser change;
 *  the browser-local copy renders until that answer arrives and remains the fallback when the
 *  endpoint cannot be reached. Panel collapse is a reading preference and stays local. */
export interface DashboardLayout { left: string[]; center: string[]; right: string[]; hidden: string[] }
/** The operator's notes scratchpad (`GET`/`POST /api/notes`, #730): one plain-text document, saved
 *  on the server so it survives a browser change, with this browser's copy (`svan-notes:v1`) as what
 *  renders before the answer arrives and what stands when the endpoint cannot be reached. `GET`
 *  answers `null` when nothing is stored — the client then keeps the copy it remembers. */
export interface DashboardNotes { text: string }
export interface Hand { id:number; hand_id:string; ts:number; hole:string[]; board:string[]; net:number | null; big_blind:number; version:string }
export interface PositionSegments { early: { vpip: Estimate | null; pfr: Estimate | null }; late: { vpip: Estimate | null; pfr: Estimate | null } }
export interface Opponent { style?: string; advice?: string; name: string; evidence_hands?: number; vpip?: Estimate; pfr?: Estimate; aggression?: Estimate; fold_to_bet?: Estimate; by_position?: PositionSegments }
export interface OpponentTrend { hand_id: string; ts: number; net: number | null; direct: number; cumulative: number; result: string }
export interface OpponentDetail extends Opponent { head_to_head_hands: number; hero_wins: number; opponent_wins: number; won_from: number; lost_to: number; table_net: number; trend: OpponentTrend[] }
export interface ReplayEvent { type:string; ts:number; data:Record<string, unknown> }
export interface SetupSlot { slot: number; name: string; key_hint: string; enabled: boolean }
export interface SetupState { bots: SetupSlot[]; buy_in: number; seek_top_rank: number; max_bots: number; can_write: boolean; write_blocked: string | null; supervised: boolean; restart_pending: boolean }
export interface MonitorLine { time: string; kind: string; text: string }
export interface MonitorState { monitor: { running: boolean; log_age_seconds: number | null; started: MonitorLine | null; summary: MonitorLine | null; opponents: MonitorLine | null; alerts: MonitorLine[] }; pressure: { cpu: number | null; io: number | null; memory: number | null }; replays: { recorded: number | null; error?: string | null; newest: string | null; keep_days: number }; season_check: { result: string; failures: string[] } | null }
export interface ChangelogEntry { commit: string; subject: string; group: string }
export interface UpdateCheck { source: string; commit: string | null; behind: number; checked_at: number; fetched_at?: number | null; error: string | null; branch?: string | null; ahead?: number | null }
export interface ReleasesState { installed: { commit: string | null; at: string | null; subject: string | null }; head: { commit: string | null; subject: string | null }; behind: number; dirty: boolean; update_available: boolean; build: { commit: string; version: string }; changelog: ChangelogEntry[]; remote?: UpdateCheck | null }
export interface ReleaseLog { running: boolean; log: string[] }
/** GET /api/host (0241): one host fact, judged; `advice` is the operator's command when `warn`. */
export interface HostCheck { key: string; label: string; value: string; status: 'ok' | 'warn' | 'info'; advice: string | null }
export interface HostState { checks: HostCheck[]; checked_at: number }
/** A saved build; `readable` is false when it cannot read the compressed databases (0229). */
export interface SavedBuild { commit: string; subject: string | null; installed_at: string | null; current: boolean; readable?: boolean }
export type StageState = 'pending' | 'running' | 'done' | 'failed';
export interface ReleaseStage { name: string; state: StageState; seconds: number; expected: number }
/** GET /api/releases/progress (0236): one update run, weighted by the last run's stage times. */
export interface ReleaseProgress {
  /** `current`: nothing to install — the checkout is already ahead of the update branch (#394). */
  state: 'idle' | 'running' | 'failed' | 'installed' | 'current';
  running: boolean;
  stages: ReleaseStage[];
  percent: number;
  elapsed: number | null;
  /** When a finished run ended, so a completed update can say when, not just how long (#320). */
  finished_at?: number | null;
  eta: number | null;
  from: string | null;
  commit: string | null;
  message: string | null;
  swap: { target: string | null; fleet: string; fleet_done: boolean; learner: string | null; analyst: string | null; workers: { bot: string; commit: string | null }[] };
  bots_playing: number;
  bots_total: number;
  log: string[];
}
export interface HealthState { ok: boolean; version: string; commit: string }

/** The rates a player card shows, ours and the league's (shrunk toward the population). */
export interface Rates { vpip:number; pfr:number; open_raise:number; limp:number; three_bet:number; call_open:number; fold_to_3bet:number; four_bet:number; fold_to_4bet:number; cbet:number; fold_to_cbet:number; wtsd:number; won_showdown:number; river_bluff:number; bet_first:number[]; fold_vs_bet:number[]; raise_vs_bet:number[]; vpip_pos:number[]; open_pos:number[] }
/** One hand we shared with a player, as `vs_us.biggest_win`, `vs_us.biggest_loss` and `vs_us.recent`
 *  report it. The `bot` names the live seat (`Shared::current_name`), retired names resolved. */
export interface KeyHand { hand_id: string; bot: string; net: number; pot: number; ts: number }
/** One recorded season finish of a player, from the server's reputation store. */
export interface ReputationFinish { season: number; rank: number; score: number; hands: number; participants: number }
/** What the server knows about a player across seasons (the same store the Rivals panel reads). */
export interface Reputation {
  best_rank?: number | null; current_rank?: number | null; seasons?: number | null; top10?: number | null;
  strength?: number | null; lifetime_hands?: number | null; names?: string[] | null; finishes?: ReputationFinish[] | null;
}
/** Public leaderboard standing (`/api/leaderboard`); `rank_delta` is movement since the last refresh. */
export interface Standing { rank?: number | null; score?: number | null; hands?: number | null; win_rate?: number | null; rank_delta?: number | null; score_delta?: number | null; pro?: boolean; ours?: boolean }
/** `GET /api/players/{name}/card` (0217): the one payload behind the scout view (0296). */
export interface PlayerCard {
  name: string; ours?: boolean; avatar_url?: string | null; style: string; advice: string; hands_observed: number; confidence: number;
  read: Rates; league: Rates;
  corrections: { fold_offset?: number | null; response_ratio?: number[] | null; size_tell?: number | null };
  leaderboard?: Standing | null;
  reputation?: Reputation | null;
  /** The chip flow attributed to this player's seat, the number every other surface shows (0280). */
  vs_seat?: { hands:number; bb_per_100:number; low_95:number; high_95:number; beats_us:boolean; we_beat:boolean } | null;
  vs_us: { hands:number;
    /** Hands with a positive recorded blind used for bb/100, when the API supplies this count. */
    priced_hands?:number; net:number; ev_net:number; bb100:number|null; confidence:number|null; ev_bb100:number|null; ev_confidence:number|null; won_pots:number; lost_pots:number;
    biggest_win?: KeyHand | null; biggest_loss?: KeyHand | null; by_bot: {bot:string;hands:number;net:number}[]; form: string[]; series: {hand:number;net:number;ev:number}[];
    /** The newest shared hands with a result, newest first, when the server sends them; without
     *  them the card names only the two extremes. */
    recent?: KeyHand[] | null };
}

/** `GET /api/analysis` (leak finder): every recorded hand grouped into leaks. Rates use each
 *  hand's own recorded blind; `priced_hands` counts the hands that had a positive one, when the
 *  API supplies the count. Older payloads omit it and report every rate over all hands. */
export interface LeakLineRow { key: string; label: string; hands: number; /** Hands with a positive recorded blind behind the bb rates, when supplied. */ priced_hands?: number; total_chips: number; bb_per_hand: number | null; low_bb: number | null; high_bb: number | null; share_bb100: number | null }
export interface LeakResult { hands?: number; priced_hands?: number; bb100: number | null; low_bb100: number | null; high_bb100: number | null; chips: number }
export interface LeakSeasonResult extends LeakResult { scoped: boolean; number: number | null; started_at: number | null }
export interface LeakTrendBlock { block: number; hands_end: number; priced_hands?: number; bb100: number | null; cumulative_bb: number | null; until?: string | null }
export interface LeakOpponent { name: string; hands: number; priced_hands?: number; bb100: number | null; upper_bb100: number | null; beats_us: boolean }
export interface LeakTrip { street: string; bet_into_us: number; bet_into_us_n: number; bet_elsewhere: number; bet_elsewhere_n: number; z: number; our_fold: number; faced: number; mdf_fold: number; flagged: boolean }
export interface LeakSuggestion { severity: 'high' | 'medium' | 'info'; title: string; evidence: string; action: string }
export interface LeakAnalysis {
  hands: number; priced_hands?: number;
  /** This season's result, and which season that is. */
  season?: LeakSeasonResult;
  /** Every recorded hand: the leaks themselves are learning and carry across seasons. */
  overall: LeakResult;
  costly_lines: LeakLineRow[]; best_lines: LeakLineRow[]; outcomes: LeakLineRow[]; positions: LeakLineRow[];
  trend: LeakTrendBlock[];
  tripwires: LeakTrip[];
  opponents: LeakOpponent[];
  suggestions: LeakSuggestion[];
}

/** One per-opponent fit on the Per-opponent reads panel (`GET /api/intel`, 0222). */
export interface IntelFit {
  id: 'fold' | 'response' | 'sizing';
  title: string;
  reads: string;
  stored: boolean;
  active: boolean;
  evidence: { gain_mnats: number; half_width_mnats: number; n: number | null } | null;
  installed: number;
}
export interface IntelOpponent { name: string; hands: number; fold_offset: number | null; response_ratio: number[] | null; size_tell: number | null }
export interface IntelState { fits: IntelFit[]; corrected_opponents: number; opponents: IntelOpponent[] }

/** How the analyst graded one decision (0220): the deep re-solve's best answer against the live one. */
export type Grade = 'best' | 'good' | 'inaccuracy' | 'mistake' | 'blunder';
/** One graded decision, as `/api/accuracy`'s `worst[]` reports the biggest of them. */
export interface GradedDecision { ts: number; bot: string; hand_id: string; street: string; live_action: string; deep_action: string; loss_bb: number; pot_bb: number; grade: Grade }
/** A bot's (or the fleet's) grades over the window, with Lichess's accuracy formula. */
export interface DecisionReport { decisions: number; accuracy: number; grades: number[]; mean_loss_bb: number }
/** `GET /api/accuracy` (0220): the analyst's grades for the last `days`, the worst of them, and the quiz feed. */
export interface AccuracyState { days: number; fleet: DecisionReport; bots: { bot: string; report: DecisionReport }[]; streets: { street: string; report: DecisionReport }[]; worst: GradedDecision[] }

/** One live component's measured value (0316): the share of recorded big decisions that move with it
 * switched off, and what the moved choice gives up under the full model. */
export interface WiringRow { component: string; changed: number; share_pct: number; cost_bb: number; max_bb: number; /** Spots that had the component to switch off (#315); absent on older rows. */ installed?: number | null }
/** The analyst's wiring measurement (`review_wiring::WiringReport`, store row `wiring.v1`). */
/** Per street, how many of every decision in the last day the self-calibration bias decided (0332). */
export interface CalibrationFlips { street: string; decisions: number; flipped: number; share_pct: number; main: string; main_count: number }
export interface WiringReport { at: number; sample: number; unstable: number; exact: number; exact_with_current: number; carrying_corrections: number; v3: number; v3_exact: number; chosen_not_best: number; rows: WiringRow[]; calibration?: CalibrationFlips[]; /** The sample per street, in play order (#315). */ streets?: [string, number][] }
/** `GET /api/wiring` (0316): the stored measurement with its age, or why there is none — never an empty table. */
export type WiringState = { available: false; reason: string } | { available: true; age_secs: number; stale: boolean; report: WiringReport };

/** `GET /api/experiment` (0291): experiment mode of the two trailing bots. */
export interface ExperimentArmStats { hands: number; mean_bb: number; se_bb: number }
export interface ExperimentEstimate { treatment: ExperimentArmStats; control: ExperimentArmStats; diff_bb100: number; lower_bb100: number; upper_bb100: number; effective_hands: number }
export interface ExperimentMode {
  status: 'champion' | 'active_no_target' | 'running';
  season: string | null;
  qualifying: { readings: number; needed: number };
  protected: string[];
  pair: string[];
  ranks: Record<string, number>;
  last_reading_at: number | null;
  last_attempt_at: number | null;
  reading_age_secs: number | null;
  stale_after_secs: number;
  last_error: string | null;
  last_transition: { at: number; to: 'champion' | 'active'; reason: string } | null;
  reason: string | null;
  bots: { name: string; role: 'treatment' | 'champion control' | null; hands_on_target: number; last_assignment_change: number | null }[];
  target: null | { id: string; label: string; knob: string; old: number; new: number; hypothesis: string; source: 'confirmation' | 'ledger'; sim: { hands: number; mean_bb100: number; upper_bb100: number }; champion: string | null; population_watermark: number | null; estimate: ExperimentEstimate | null; required_hands: number; min_hands: number; next_gate: string | null; safety_bound: string; review: string };
  verdicts: { id: string; label: string; verdict: 'live-supported' | 'live-harmful' | 'inconclusive'; at: number; estimate: ExperimentEstimate }[];
}
