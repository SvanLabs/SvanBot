import { useEffect, useRef, useState } from 'react';
import { ArrowUpCircle, Check, CircleDashed, History, LoaderCircle, RefreshCw, TriangleAlert, X } from 'lucide-react';
import type { HealthState, ReleaseProgress, ReleasesState, ReleaseStage, SavedBuild } from './types';
import { request } from './api';
import { time } from './format';

const call = request;

const STAGE_LABEL: Record<string, string> = {
  fetch: 'Download from GitHub',
  snapshot: 'Save the current build',
  lint: 'Lint',
  build: 'Lint, test build and release build',
  test: 'Test',
  dashboard: 'Dashboard',
  install: 'Install',
  restore: 'Restore saved build',
};

function clock(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return '—';
  const s = Math.max(0, Math.round(seconds));
  return s >= 60 ? `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s` : `${s}s`;
}

function ago(epochSecs: number | null | undefined): string {
  if (!epochSecs) return 'never';
  const s = Math.max(0, Date.now() / 1000 - epochSecs);
  return s < 90 ? 'just now' : s < 5400 ? `${Math.round(s / 60)} min ago` : `${Math.round(s / 3600)} h ago`;
}

/** What a failed check means for the operator, in their terms (#320). The tool's own wording — the
 *  last line of `scripts/update.sh --check`'s stderr, kept in the title and the log — names the
 *  mechanism ("could not fetch origin/main (network or credentials)"), which is not a next step. */
function checkFailFix(error: string): string {
  const e = error.toLowerCase();
  if (/could not fetch|resolve host|network|timed out|connection|unreachable/.test(e)) {
    return 'GitHub could not be reached from this host — no network, or it is offline. The fleet keeps playing the installed build, and the next automatic check retries in 30 minutes.';
  }
  if (/authentication|username|password|403|401|permission|credential|publickey/.test(e)) {
    return 'GitHub refused this checkout\'s credentials, so no update can fetch until an operator signs the host in again. The fleet keeps playing the installed build.';
  }
  if (/not a git repository|no such file|not found/.test(e)) {
    return 'The checkout is not readable from here, so the update check cannot run on this host. The fleet keeps playing the installed build.';
  }
  return 'The update check did not answer — the tool\'s own words are in the title. The fleet keeps playing the installed build, and the next automatic check retries in 30 minutes.';
}

function StageIcon({ stage }: { stage: ReleaseStage }) {
  if (stage.state === 'done') return <Check size={13} aria-hidden/>;
  if (stage.state === 'running') return <LoaderCircle size={13} className="spin" aria-hidden/>;
  if (stage.state === 'failed') return <TriangleAlert size={13} aria-hidden/>;
  return <CircleDashed size={13} aria-hidden/>;
}

/** One update run: a stage-weighted bar with time left, the stages, play status and the swap (0236). */
export function UpdateProgress({ progress }: { progress: ReleaseProgress }) {
  const { state, percent, stages, swap } = progress;
  const [showLog, setShowLog] = useState(state === 'failed');
  const failedStage = stages.find(s => s.state === 'failed');
  const swapped = swap.fleet_done;
  const rollback = stages.length === 1 && stages[0].name === 'restore';
  const noun = rollback ? 'Rollback' : 'Update';
  const title = state === 'running' ? (rollback ? 'Rolling back' : 'Updating') : state === 'failed' ? `${noun} failed` : state === 'installed' ? (swapped ? `${noun} complete` : 'Installed — swapping in') : state === 'current' ? 'Nothing to install' : 'No update running';
  const total = stages.length;
  const doneCount = stages.filter(s => s.state === 'done').length;
  return <section className={`update-progress ${state}`} aria-label="Update progress">
    <div className="update-progress-head">
      <strong>{title}</strong>
      <span className="mono">{state === 'running' ? `${percent.toFixed(0)}% · ${clock(progress.elapsed)} elapsed · about ${clock(progress.eta)} left` : state === 'installed' ? `${progress.from ?? '?'} → ${progress.commit ?? '?'} in ${clock(progress.elapsed)}${progress.finished_at ? ` · finished ${ago(progress.finished_at)}` : ''}` : state === 'failed' ? `stopped after ${clock(progress.elapsed)}${progress.finished_at ? ` · ${ago(progress.finished_at)}` : ''}` : ''}</span>
    </div>
    {/* Nothing ran and nothing will: no bar, no stages, no hot swap — the message says why (#394). */}
    {state !== 'current' && <>
    <div className="update-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent)} aria-label="Update progress">
      <div style={{ width: `${Math.max(2, percent)}%` }}/>
    </div>
    <ol className="update-stages" aria-label={`Stage ${Math.min(doneCount + 1, total)} of ${total}`}>
      {stages.map(s => <li key={s.name} className={s.state} title={s.state === 'done' ? `took ${clock(s.seconds)} (usually ${clock(s.expected)})` : s.state === 'running' ? `${clock(s.seconds)} of about ${clock(s.expected)}` : `usually ${clock(s.expected)}`}>
        <StageIcon stage={s}/><span>{STAGE_LABEL[s.name] ?? s.name}</span>{s.state === 'done' || s.state === 'running' ? <small className="mono">{clock(s.seconds)}</small> : null}
      </li>)}
      <li className={state === 'installed' ? (swapped ? 'done' : 'running') : 'pending'} title="The fleet swaps to the new build between turns, then the learner and analyst between their cycles">
        {state === 'installed' ? (swapped ? <Check size={13} aria-hidden/> : <LoaderCircle size={13} className="spin" aria-hidden/>) : <CircleDashed size={13} aria-hidden/>}<span>Hot swap</span>
      </li>
    </ol>
    <div className="update-live">
      <span className={progress.bots_playing > 0 ? 'positive' : 'amber'}>● {progress.bots_playing} of {progress.bots_total} bots playing{state === 'running' ? ' — play continues during the update' : ''}</span>
      {state === 'installed' && <span className="mono">fleet {swapped ? '✓' : '…'} {swap.fleet}{swap.learner ? ` · learner ${swap.learner === swap.target ? '✓' : 'next cycle'}` : ''}{swap.analyst ? ` · analyst ${swap.analyst === swap.target ? '✓' : 'next batch'}` : ''}{swap.workers.length ? ` · workers ${swap.workers.filter(w => w.commit === swap.target).length}/${swap.workers.length}` : ''}</span>}
    </div>
    </>}
    {state === 'failed' && <p className="footnote negative">{failedStage ? `Failed at ${STAGE_LABEL[failedStage.name] ?? failedStage.name}. ` : ''}{progress.message ?? 'The update did not install.'} The fleet keeps playing the installed build.</p>}
    {state === 'current' && <p className="footnote">{progress.message ?? 'This checkout is already ahead of the update branch.'} The fleet keeps playing the installed build.</p>}
    <button className="text-button" onClick={() => setShowLog(v => !v)} aria-expanded={showLog}>{showLog ? 'Hide' : 'Show'} log</button>
    {showLog && <div className="log-list update-log">{progress.log.map((l, i) => <div className="log-entry" key={i}><p className="mono">{l}</p></div>)}</div>}
  </section>;
}

/** Releases and one-click updates: what an update would bring from GitHub, the guarded trigger, and
 * live progress through build, tests, install and the hot swap. */
export function UpdatesPanel() {
  const [data, setData] = useState<ReleasesState | null>(null);
  const [failed, setFailed] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [checking, setChecking] = useState(false);
  const [progress, setProgress] = useState<ReleaseProgress | null>(null);
  const [message, setMessage] = useState<{kind: 'ok' | 'error'; text: string} | null>(null);
  const [, setHealthCommit] = useState<string | null>(null);
  const [reloadPrompt, setReloadPrompt] = useState(false);
  const polling = useRef(false);
  const [saved, setSaved] = useState<SavedBuild[]>([]);
  const [rollbackTo, setRollbackTo] = useState<SavedBuild | null>(null);
  const loadSaved = () => call<{snapshots: SavedBuild[]}>('/releases/snapshots').then(v => setSaved(v.snapshots)).catch(() => {});

  const load = () => call<ReleasesState>('/releases').then(v => { setData(v); setFailed(false); }).catch(() => setFailed(true));
  const loadProgress = () => call<ReleaseProgress>('/releases/progress').then(v => { setProgress(v); return v; }).catch(() => null);
  useEffect(() => {
    load();
    loadSaved();
    // A reload mid-update picks the run up again: progress lives on the server.
    loadProgress();
    const id = window.setInterval(load, 30_000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    let alive = true;
    const id = window.setInterval(() => {
      call<HealthState>('/health').then(h => {
        if (!alive) return;
        setHealthCommit(prev => {
          if (prev !== null && prev !== h.commit) setReloadPrompt(true);
          return h.commit;
        });
      }).catch(() => {});
    }, 30_000);
    return () => { alive = false; window.clearInterval(id); };
  }, []);

  // Poll fast while a run is going or its swap is pending; the fleet's own restart during the swap
  // only makes a few polls fail.
  const active = !!progress && (progress.running || progress.state === 'running' || (progress.state === 'installed' && !progress.swap.fleet_done));
  useEffect(() => {
    if (!active || polling.current) return;
    polling.current = true;
    const id = window.setInterval(() => {
      loadProgress().then(v => {
        if (v && v.state !== 'running' && !v.running && (v.state !== 'installed' || v.swap.fleet_done)) {
          window.clearInterval(id);
          polling.current = false;
          load();
          loadSaved();
        }
      });
    }, 2_000);
    return () => { window.clearInterval(id); polling.current = false; };
  }, [active]);

  const start = async () => {
    setMessage(null);
    try {
      await call<{started: boolean}>('/releases/update', {});
      setConfirming(false);
      await loadProgress();
    } catch (e) {
      setMessage({kind: 'error', text: (e as Error).message});
    }
  };

  const rollBack = async (build: SavedBuild) => {
    setMessage(null);
    try {
      await call<{started: boolean}>('/releases/rollback', {commit: build.commit});
      setRollbackTo(null);
      await loadProgress();
    } catch (e) {
      setMessage({kind: 'error', text: (e as Error).message});
    }
  };

  const check = async () => {
    setChecking(true);
    setMessage(null);
    try {
      await call('/releases/check', {});
      await load();
    } catch (e) {
      setMessage({kind: 'error', text: (e as Error).message});
    } finally {
      setChecking(false);
    }
  };

  if (!data) return <p className="footnote">{failed ? 'Release data unavailable.' : 'Loading releases…'}</p>;
  const remote = data.remote;
  const pending = data.behind;
  const groups: [string, typeof data.changelog][] = [];
  for (const entry of data.changelog) {
    const group = groups.find(g => g[0] === entry.group);
    if (group) group[1].push(entry);
    else groups.push([entry.group, [entry]]);
  }
  // `current` sits with `failed` and `installed`: the run is over, so nothing polls it, but its
  // message is the whole point of the state and has to stay on screen (#394).
  const showProgress = progress && progress.state !== 'idle' && (active || progress.state === 'failed' || progress.state === 'installed' || progress.state === 'current');
  return <div className="updates-panel">
    {reloadPrompt && <div className="connection-banner" role="alert">The dashboard backend updated — <button className="text-button" onClick={() => window.location.reload()}>reload to match it</button>.</div>}
    <div className="health-list">
      <div><span>Installed</span><b className="mono">{data.installed.commit || 'unknown'}{data.installed.subject ? ` · ${data.installed.subject.length > 60 ? `${data.installed.subject.slice(0, 59)}…` : data.installed.subject}` : ''}</b></div>
      <div><span>Live build</span><b className="mono">{data.build.commit} · v{data.build.version}</b></div>
      <div><span>GitHub ({remote?.source ?? 'origin/main'})</span><b className={remote?.error ? 'negative' : pending ? 'amber' : 'positive'} title={remote?.error ?? ''}>{remote?.error ? 'check failed' : pending ? `${pending} update${pending === 1 ? '' : 's'} to install` : 'Up to date'}<small className="subtle"> · checked {ago(remote?.checked_at)}</small></b></div>
      {/* `Next update`, not `Checkout`: this is what the next run cannot do, while the card below is
          the run that already finished (#320). On a fleet host the checkout is a playing machine, so
          the tooltip says what the operator can actually do about it. */}
      {data.dirty && <div><span>Next update</span><b className="negative" title="An update builds from this checkout and will not start while crates/, web/ or Cargo files have uncommitted changes — the binary would not match any commit. Commit or discard them, or run scripts/release.sh by hand. The fleet keeps playing the installed build.">Blocked — uncommitted build inputs</b></div>}
    </div>
    {showProgress && progress && <UpdateProgress progress={progress}/>}
    {data.update_available && !active && <div className="update-banner"><ArrowUpCircle size={15}/><span>{pending} commit{pending === 1 ? '' : 's'} since <b className="mono">{data.installed.commit}</b> — one click downloads, builds, tests and installs them; the bots keep playing and swap between turns.</span><button className="button primary" disabled={data.dirty} onClick={() => setConfirming(true)}>Update</button></div>}
    <div className="settings-row"><span className="footnote">Checked for updates every 30 minutes.</span><button className="button" disabled={checking || active} onClick={check}><RefreshCw size={12} className={checking ? 'spin' : ''}/>{checking ? 'Checking…' : 'Check now'}</button></div>
    {remote?.error && <p className="footnote negative" title={remote.error}>Update check failed. {checkFailFix(remote.error)}</p>}
    {groups.length > 0 && <section aria-label="Changelog since installed"><h3>What the update brings (since {data.installed.commit})</h3>
      {/* The list is read from the local refs, so it is only as fresh as the last fetch that worked:
          after a failed check it must not read as today's (#320). */}
      {remote?.error && <p className="footnote">This list is the one the last successful check fetched{remote.fetched_at ? ` — ${ago(remote.fetched_at)}` : ''}. Today's commits are not in it.</p>}
      {groups.map(([group, entries]) => <div key={group}><h4 className="changelog-group">{group}</h4><ul className="changelog-list">{entries.map(e => <li key={e.commit}><span className="mono">{e.commit}</span> {e.subject}</li>)}</ul></div>)}
    </section>}
    {!groups.length && !data.update_available && <p className="footnote">Installed build is the newest{data.installed.at ? ` (installed ${time(data.installed.at, { date: true })})` : ''}.</p>}
    {message && <p className={`footnote ${message.kind === 'error' ? 'negative' : 'positive'}`}>{message.text}</p>}
    {saved.some(b => !b.current) && <details className="saved-builds">
      <summary><History size={13}/> Roll back to a saved build ({saved.filter(b => !b.current).length})</summary>
      <p className="footnote">Every update saves the build it replaces. Rolling back reinstalls one after checking its hashes; the bots keep playing and swap to it between turns. A later Update brings the newest build back.</p>
      <ul className="changelog-list">{saved.filter(b => !b.current).map(b => <li key={b.commit}><span className="mono">{b.commit}</span> {b.subject ?? 'saved build'}{b.installed_at ? <small className="subtle"> · installed {time(b.installed_at, { date: true })}</small> : null} {b.readable === false ? <small className="subtle" title="This build reads only uncompressed data: stop the fleet, run ./target/release/archive unpack, then roll back."> · reads only uncompressed data</small> : <button className="text-button" disabled={active} onClick={() => setRollbackTo(b)}>Roll back</button>}</li>)}</ul>
    </details>}
    {rollbackTo && <div className="modal-backdrop" onClick={() => setRollbackTo(null)}><div className="modal" role="dialog" aria-label="Confirm rollback" onClick={e => e.stopPropagation()}>
      <div className="modal-heading"><div><span className="eyebrow">CONFIRM ROLLBACK</span><h2>Reinstall {rollbackTo.commit}?</h2></div><button className="icon-button" aria-label="Close" onClick={() => setRollbackTo(null)}><X size={16}/></button></div>
      <p className="footnote">{rollbackTo.subject ?? 'Saved build'}. Replaces the installed {data.installed.commit} after verifying the saved files; the bots keep playing and swap between turns.</p>
      <div className="settings-row"><strong>Roll back now?</strong><span><button className="button" onClick={() => setRollbackTo(null)}>Cancel</button> <button className="button primary" onClick={() => rollBack(rollbackTo)}><History size={12}/>Roll back</button></span></div>
    </div></div>}
    {confirming && <div className="modal-backdrop" onClick={() => setConfirming(false)}><div className="modal" role="dialog" aria-label="Confirm update" onClick={e => e.stopPropagation()}>
      <div className="modal-heading"><div><span className="eyebrow">CONFIRM UPDATE</span><h2>Install {remote?.commit ?? data.head.commit}?</h2></div><button className="icon-button" aria-label="Close" onClick={() => setConfirming(false)}><X size={16}/></button></div>
      <div className="settings-details">
        <div><span>From</span><b className="mono">{data.installed.commit}</b></div>
        <div><span>To</span><b className="mono">{remote?.commit ?? data.head.commit}</b></div>
        <div><span>Commits</span><b>{pending}</b></div>
      </div>
      <p className="footnote">Downloads from GitHub, then builds and tests at low priority (usually a few minutes). The bots keep playing the whole time and swap to the new build between turns. If any step fails, nothing is installed.</p>
      <div className="settings-row"><strong>Install now?</strong><span><button className="button" onClick={() => setConfirming(false)}>Cancel</button> <button className="button primary" onClick={start}><RefreshCw size={12}/>Update</button></span></div>
    </div></div>}
  </div>;
}
