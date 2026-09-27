import React, { useMemo, useState } from 'react';
import { BookOpen, Search, X } from 'lucide-react';
import source from '../../docs/GUIDE.md?raw';
import { DocsDiagram } from './docs-diagrams';

type Block =
  | { kind: 'h'; level: number; text: string; id: string }
  | { kind: 'p'; text: string }
  | { kind: 'ul' | 'ol'; items: string[] }
  | { kind: 'code'; text: string; lang: string }
  | { kind: 'table'; head: string[]; rows: string[][] }
  | { kind: 'hr' };

const slug = (t: string) => t.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/(^-|-$)/g, '');

function parse(md: string): Block[] {
  const lines = md.split('\n');
  const out: Block[] = [];
  let i = 0;
  const cells = (l: string) => l.trim().replace(/^\||\|$/g, '').split(/(?<!\\)\|/).map(c => c.trim());
  while (i < lines.length) {
    const line = lines[i];
    if (line.startsWith('```')) {
      const lang = line.slice(3).trim();
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith('```')) body.push(lines[i++]);
      out.push({ kind: 'code', text: body.join('\n'), lang });
      i++;
      continue;
    }
    const h = /^(#{1,4})\s+(.*)$/.exec(line);
    if (h) { out.push({ kind: 'h', level: h[1].length, text: h[2], id: slug(h[2]) }); i++; continue; }
    if (/^---+\s*$/.test(line)) { out.push({ kind: 'hr' }); i++; continue; }
    if (line.trim().startsWith('|') && i + 1 < lines.length && /^\s*\|[\s:|-]+\|\s*$/.test(lines[i + 1])) {
      const head = cells(line);
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && lines[i].trim().startsWith('|')) rows.push(cells(lines[i++]));
      out.push({ kind: 'table', head, rows });
      continue;
    }
    if (/^\s*[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && (/^\s*[-*]\s+/.test(lines[i]) || (/^\s{2,}\S/.test(lines[i]) && items.length))) {
        if (/^\s*[-*]\s+/.test(lines[i])) items.push(lines[i].replace(/^\s*[-*]\s+/, ''));
        else items[items.length - 1] += ' ' + lines[i].trim();
        i++;
      }
      out.push({ kind: 'ul', items });
      continue;
    }
    if (/^\s*\d+\.\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && (/^\s*\d+\.\s+/.test(lines[i]) || (/^\s{2,}\S/.test(lines[i]) && items.length))) {
        if (/^\s*\d+\.\s+/.test(lines[i])) items.push(lines[i].replace(/^\s*\d+\.\s+/, ''));
        else items[items.length - 1] += ' ' + lines[i].trim();
        i++;
      }
      out.push({ kind: 'ol', items });
      continue;
    }
    if (!line.trim()) { i++; continue; }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^(#{1,4}\s|```|\s*[-*]\s|\s*\d+\.\s|\s*\|)/.test(lines[i])) para.push(lines[i++]);
    out.push({ kind: 'p', text: para.join(' ') });
  }
  return out;
}

function Inline({ text }: { text: string }) {
  const parts: React.ReactNode[] = [];
  const re = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*|<https?:\/\/[^>]+>|\[[^\]]+\]\([^)]+\))/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let k = 0;
  while ((m = re.exec(text))) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    const t = m[0];
    if (t.startsWith('`')) parts.push(<code key={k++}>{t.slice(1, -1)}</code>);
    else if (t.startsWith('**')) parts.push(<strong key={k++}>{t.slice(2, -2)}</strong>);
    else if (t.startsWith('*')) parts.push(<em key={k++}>{t.slice(1, -1)}</em>);
    else if (t.startsWith('<')) parts.push(<a key={k++} href={t.slice(1, -1)} target="_blank" rel="noreferrer">{t.slice(1, -1)}</a>);
    else { const mm = /\[([^\]]+)\]\(([^)]+)\)/.exec(t)!; parts.push(<a key={k++} href={mm[2]} target="_blank" rel="noreferrer">{mm[1]}</a>); }
    last = m.index + t.length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return <>{parts}</>;
}

export function DocsPage() {
  const blocks = useMemo(() => parse(source), []);
  const [query, setQuery] = useState('');
  const sections = useMemo(() => {
    const s: { id: string; title: string; blocks: Block[] }[] = [];
    for (const b of blocks) {
      if (b.kind === 'h' && b.level <= 2) s.push({ id: b.id, title: b.text, blocks: [b] });
      else if (s.length) s[s.length - 1].blocks.push(b);
    }
    return s;
  }, [blocks]);
  const q = query.trim().toLowerCase();
  const visible = q ? sections.filter(sec => JSON.stringify(sec.blocks).toLowerCase().includes(q)) : sections;
  const render = (b: Block, i: number) => {
    switch (b.kind) {
      case 'h': return React.createElement(`h${Math.min(b.level + 1, 5)}`, { key: i, id: b.id }, <Inline text={b.text} />);
      case 'p': return <p key={i}><Inline text={b.text} /></p>;
      case 'ul': return <ul key={i}>{b.items.map((it, j) => <li key={j}><Inline text={it} /></li>)}</ul>;
      case 'ol': return <ol key={i}>{b.items.map((it, j) => <li key={j}><Inline text={it} /></li>)}</ol>;
      case 'code': return b.lang.startsWith('diagram:') ? <DocsDiagram key={i} name={b.lang.slice(8)} ascii={b.text}/> : <pre key={i}><code>{b.text}</code></pre>;
      case 'hr': return <hr key={i} />;
      case 'table': return <div key={i} className="docs-table"><table><thead><tr>{b.head.map((c, j) => <th key={j}><Inline text={c} /></th>)}</tr></thead><tbody>{b.rows.map((r, j) => <tr key={j}>{r.map((c, n) => <td key={n}><Inline text={c} /></td>)}</tr>)}</tbody></table></div>;
    }
  };
  return <div className="docs">
    <aside className="docs-nav">
      <div className="docs-brand"><BookOpen size={16} /> SVANBOT DOCS</div>
      <label className="docs-search"><Search size={13} /><input aria-label="Search documentation" placeholder="Search docs…" value={query} onChange={e => setQuery(e.target.value)} />{query && <button aria-label="Clear search" onClick={() => setQuery('')}><X size={12} /></button>}</label>
      <nav>{sections.filter(s => !s.title.endsWith(' Documentation')).map(s => <a key={s.id} href={`#docs/${s.id}`} className={visible.includes(s) ? '' : 'dim'} onClick={e => { e.preventDefault(); document.getElementById(s.id)?.scrollIntoView({ behavior: 'smooth' }); }}>{s.title}</a>)}</nav>
      <a className="docs-back" href="#">← Back to control room</a>
    </aside>
    <article className="docs-body">{visible.length ? visible.map(s => <section key={s.id}>{s.blocks.map(render)}</section>) : <p>No section matches “{query}”.</p>}</article>
  </div>;
}
