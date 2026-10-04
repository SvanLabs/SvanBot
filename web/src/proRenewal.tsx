import type { ProRenewal } from './types';
import { time } from './format';

/** One Runtime-health row for the Pro tier of the owner key and the auto-renew switch (#775, #776). */
export function ProRenewalRow({ r }: { r?: ProRenewal }) {
  if (!r || !r.multi_key) return null;
  const lapsed = r.owner_tier === 'free';
  const text = lapsed
    ? r.enabled ? (r.last_error ? 'lapsed · renewal failing' : 'lapsed · renewing') : 'lapsed · renew at openpoker.ai'
    : r.owner_tier === 'pro' ? (r.enabled ? 'Pro · auto-renew on' : 'Pro · auto-renew off') : 'unknown';
  const detail = [
    r.receipt ? `Last renewal ${time(r.receipt.at)}: ${r.receipt.seasons_purchased} season(s), ${r.receipt.seasons_remaining} remaining.` : 'No renewal made by this box.',
    r.last_error ? `Last error: ${r.last_error}` : '',
    r.enabled ? `Buys up to ${r.max_seasons} seasons from credits.` : 'Set SVANBOT_AUTO_RENEW_PRO=1 in .env to renew from credits.',
  ].filter(Boolean).join(' ');
  return <div><span>Pro</span><b className={lapsed ? 'negative' : r.owner_tier === 'pro' ? 'positive' : 'muted'} title={detail}>{text}</b></div>;
}
