pub const FONTS: &str = "https://fonts.googleapis.com/css2?family=Big+Shoulders+Display:wght@600;700&family=Hanken+Grotesk:wght@400;600&family=Martian+Mono:wght@400;500&display=swap";

/// Design system "TradingTime" (timeperp tokens): night-watch dark by default, almanac light via `.light`.
pub const CSS: &str = r#"
.tt { --ground:#0e1120; --surface:#171b2e; --ink:#e8eaf3; --muted:#9ca3bf; --rule:#3a4060;
  --calm:#7aa2ff; --on-calm:#0e1120; --wild:#ff8f4f; --on-wild:#0e1120; --gain:#45c29b; --loss:#ff7a6e;
  --font-display:"Big Shoulders Display","Arial Narrow",sans-serif; --font-sans:"Hanken Grotesk","Segoe UI",system-ui,sans-serif;
  --font-mono:"Martian Mono",ui-monospace,"SF Mono",Menlo,monospace; color-scheme:dark; }
.tt.light { --ground:#f1f2f6; --surface:#fbfbfd; --ink:#161a2e; --muted:#545b78; --rule:#c9cdda;
  --calm:#1f5fd1; --on-calm:#ffffff; --wild:#b8460f; --on-wild:#ffffff; --gain:#0b7354; --loss:#c0352b; color-scheme:light; }
html, body { margin:0; background:#0e1120; }
.tt { min-height:100vh; box-sizing:border-box; background:var(--ground); color:var(--ink); font-family:var(--font-sans);
  font-size:13px; line-height:20px; display:flex; flex-direction:column; gap:4px; padding-bottom:4px; }
.tt * { box-sizing:border-box; }
.tt button, .tt input, .tt select { font:inherit; color:inherit; }
.tt button:focus-visible, .tt input:focus-visible, .tt select:focus-visible, .tt a:focus-visible { outline:2px solid var(--calm); outline-offset:2px; }
.tt a { color:var(--calm); } .tt a:hover { color:var(--ink); }
.num { font-family:var(--font-mono); font-variant-numeric:tabular-nums; }
.label, .lbl { font-size:11px; line-height:16px; font-weight:600; letter-spacing:.06em; text-transform:uppercase; color:var(--muted); }
.calm { color:var(--calm); } .wild { color:var(--wild); } .gain { color:var(--gain); } .loss { color:var(--loss); } .muted { color:var(--muted); }
.grow { flex:1; }

/* panels */
.panel { margin:0 4px; background:var(--surface); border-radius:2px; min-width:0; }
.phead { display:flex; flex-wrap:wrap; justify-content:space-between; align-items:center; gap:8px; padding:6px 12px; min-height:41px;
  border-bottom:1px solid var(--rule); }
.phead .title { font-weight:600; }
.phead .meta { font-size:12px; color:var(--muted); }
.pnote { margin:0; padding:8px 12px; font-size:12px; line-height:16px; color:var(--muted); }

/* controls */
.btn { height:36px; padding:0 16px; border-radius:6px; border:1px solid var(--muted); background:transparent; color:var(--ink);
  font-weight:600; cursor:pointer; white-space:nowrap; }
.btn.primary, .connect { height:36px; padding:0 16px; border-radius:6px; border:1px solid var(--calm); background:var(--calm);
  color:var(--on-calm); font-weight:600; cursor:pointer; white-space:nowrap; }
.btn.ghost { border-color:transparent; color:var(--muted); font-weight:400; } .btn.ghost:hover { color:var(--ink); }
.btn.sm { height:28px; padding:0 10px; border-radius:2px; font-size:12px; font-weight:400; }
.chips { display:flex; gap:4px; overflow-x:auto; scrollbar-width:none; min-width:0; max-width:100%; }
.phead > * { min-width:0; }
.chip, .seg button, .toggle { height:28px; padding:0 10px; border-radius:2px; border:1px solid transparent; background:transparent;
  color:var(--muted); font-family:var(--font-mono); font-size:12px; cursor:pointer; white-space:nowrap; }
.chip:hover, .seg button:hover, .toggle:hover { color:var(--ink); }
.chip.on, .seg button.on, .toggle.on { border-color:var(--ink); color:var(--ink); }
.toggle { border-color:var(--muted); font-family:var(--font-sans); }
.seg { display:flex; gap:4px; overflow-x:auto; scrollbar-width:none; }
.tabs { display:flex; gap:20px; padding:0 12px; border-bottom:1px solid var(--rule); overflow-x:auto; scrollbar-width:none; }
.tabs button { height:36px; padding:0; border:none; border-bottom:2px solid transparent; border-radius:0; background:transparent;
  color:var(--muted); cursor:pointer; white-space:nowrap; }
.tabs button.on { border-bottom-color:var(--calm); color:var(--ink); font-weight:600; }
.field { display:flex; flex-direction:column; gap:4px; }
.tt input { height:40px; width:100%; padding:0 12px; border-radius:2px; border:1px solid var(--muted); background:var(--ground);
  color:var(--ink); font-family:var(--font-mono); font-size:13px; }
.tt select { height:28px; padding:0 8px; border-radius:2px; border:1px solid var(--muted); background:var(--ground); color:var(--ink);
  font-family:var(--font-mono); font-size:12px; }
dl.kv { margin:0; display:grid; grid-template-columns:minmax(0,1fr) auto; gap:4px 12px; font-size:12px; line-height:18px; }
dl.kv dt { color:var(--muted); } dl.kv dd { margin:0; text-align:right; font-family:var(--font-mono); font-variant-numeric:tabular-nums; }

/* nav */
.nav { display:flex; flex-wrap:wrap; align-items:center; justify-content:space-between; gap:16px; padding:0 16px; min-height:52px;
  background:var(--surface); border-bottom:1px solid var(--rule); }
.nav .left, .nav .right { display:flex; align-items:center; gap:24px; flex-wrap:wrap; }
.nav .right { gap:12px; }
.wordmark { font-family:var(--font-display); font-weight:700; font-size:24px; line-height:24px; letter-spacing:.01em; }
.navlinks { display:flex; gap:24px; }
.tt a.navlink { color:var(--muted); text-decoration:none; }
.tt a.navlink:hover { color:var(--ink); }
.tt a.navlink.on { color:var(--ink); font-weight:600; }
.nav .equity { font-size:12px; color:var(--muted); }
.nav .addr { font-size:12px; color:var(--ink); }
.tabbar { display:none; }

/* market header */
.mkt { display:flex; flex-direction:column; }
.mbar { display:flex; flex-wrap:wrap; align-items:center; gap:8px 32px; padding:10px 16px; }
.mbar .stat { display:flex; flex-direction:column; }
.mbar .title { font-weight:600; font-size:15px; line-height:22px; }
.mbar .sub { font-size:12px; line-height:16px; color:var(--muted); }
.mbar .sol-price { font-size:15px; line-height:22px; font-weight:500; }
.mbar .sol .sub .num { font-size:11px; }
.mark-readout { font-family:var(--font-display); font-weight:700; font-size:28px; line-height:28px; }
.explain { margin:0; padding:8px 16px; border-top:1px solid var(--rule); color:var(--muted); }
.explain strong { font-weight:600; }

/* trade grid */
.main { margin:0 4px; display:flex; flex-wrap:wrap; gap:4px; align-items:stretch; }
.main > .panel { margin:0; }
.chart-panel { flex:999 1 560px; display:flex; flex-direction:column; }
.quotes-panel { flex:1 1 240px; max-width:300px; display:flex; flex-direction:column; }
.ticket-panel { flex:1 1 300px; max-width:360px; padding:12px; }
.legend { display:flex; gap:16px; font-size:12px; color:var(--muted); flex-wrap:wrap; }
.legend span { display:flex; align-items:center; gap:6px; }
.sw { width:8px; height:8px; border-radius:1px; display:inline-block; }
.sw.calm { background:var(--calm); } .sw.wild { background:var(--wild); }
.sw.dash { width:18px; height:0; border-top:1.5px dashed var(--muted); border-radius:0; }
svg.chart { width:100%; height:auto; display:block; padding:8px 0; }
svg.chart text { fill:var(--muted); font-family:var(--font-mono); font-size:11px; }
svg.chart .grid { stroke:var(--rule); stroke-width:1; }
svg.chart .idx { fill:none; stroke:var(--muted); stroke-width:1.5; stroke-dasharray:5 4; }
svg.chart .now { stroke:var(--ink); stroke-width:1; stroke-dasharray:2 3; }
svg.chart .now-tag { fill:var(--ink); }
svg.chart text.now-text { fill:var(--surface); }
.chart-empty { padding:40px 16px; color:var(--muted); }

/* quotes */
.lhead, .lrow { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:4px; padding:1px 12px; }
.lhead { padding:6px 12px; }
.lhead span:not(:first-child), .lrow span:not(:first-child) { text-align:right; }
.lrow { position:relative; font-family:var(--font-mono); font-variant-numeric:tabular-nums; font-size:12px; line-height:20px; cursor:pointer; }
.lrow:hover { background:color-mix(in srgb, var(--ink) 6%, transparent); }
.lrow span { position:relative; }
.lrow .bar { position:absolute; top:1px; bottom:1px; right:0; opacity:.14; }
.lrow.ask .bar { background:var(--calm); } .lrow.bid .bar { background:var(--wild); }
.lmark { display:flex; justify-content:space-between; padding:6px 12px; border-top:1px solid var(--rule); border-bottom:1px solid var(--rule); font-size:12px; }
.lmark .v { font-family:var(--font-mono); font-weight:500; }
.lempty { padding:16px 12px; color:var(--muted); font-size:12px; }

/* ticket */
.ticket { display:flex; flex-direction:column; gap:12px; }
.ticket .tabs { padding:0; gap:16px; }
.sides { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:4px; padding:3px; background:var(--ground); border-radius:6px; }
.sides button { min-height:48px; padding:6px 10px; border:none; border-radius:4px; background:transparent; color:var(--muted); cursor:pointer;
  display:flex; flex-direction:column; align-items:center; justify-content:center; }
.sides button .s { font-weight:600; } .sides button .h { font-size:11px; line-height:14px; }
.sides button.long.on { background:var(--calm); color:var(--on-calm); }
.sides button.short.on { background:var(--wild); color:var(--on-wild); }
.avail { display:flex; justify-content:space-between; font-size:12px; }
.presets { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:4px; }
.presets button { height:28px; border-radius:2px; border:1px solid var(--muted); background:transparent; color:var(--ink); font-size:12px; cursor:pointer; }
.buying { display:flex; flex-direction:column; gap:6px; padding:10px 12px; border-radius:6px; border:1px solid var(--calm); color:var(--calm); }
.buying.short { border-color:var(--wild); color:var(--wild); }
.buying .lbl { color:inherit; }
.buying p { margin:0; font-size:14px; line-height:20px; }
.buying dl.kv { gap:2px 12px; }
.buy { height:44px; border-radius:6px; border:none; font-weight:600; cursor:pointer; }
.buy.long { background:var(--calm); color:var(--on-calm); } .buy.short { background:var(--wild); color:var(--on-wild); }
.buy:disabled { opacity:.55; cursor:not-allowed; }
.txs { font-size:12px; line-height:16px; min-height:16px; overflow-wrap:anywhere; }

/* tables */
.tt table { width:100%; border-collapse:collapse; }
.tt th { padding:8px; font-size:11px; line-height:16px; font-weight:600; letter-spacing:.06em; text-transform:uppercase; color:var(--muted);
  text-align:right; white-space:nowrap; }
.tt td { padding:8px; border-top:1px solid var(--rule); font-family:var(--font-mono); font-variant-numeric:tabular-nums; font-size:12px;
  line-height:20px; text-align:right; white-space:nowrap; }
.tt th:first-child, .tt td:first-child { padding-left:12px; } .tt th:last-child, .tt td:last-child { padding-right:12px; }
.tt th.l, .tt td.l { text-align:left; }
.tt td.side, .tt td.sans { font-family:var(--font-sans); }
.tt td.side { font-weight:600; }
.acct .scroll { overflow-x:auto; }
.acct table.positions { min-width:880px; }
.empty { margin:0; padding:24px 12px; color:var(--muted); }

/* analytics */
.analytics { display:flex; flex-direction:column; gap:4px; }
.analytics .panel { padding:0; }
.kpis { margin:0 4px; display:grid; grid-template-columns:repeat(auto-fit,minmax(160px,1fr)); gap:4px; }
.kpis .panel { margin:0; padding:10px 16px; }
.kpi { display:flex; flex-direction:column; gap:2px; }
.kpi-v { font-family:var(--font-display); font-weight:700; font-size:28px; line-height:30px; }
.kpi-sub { font-size:11px; line-height:16px; color:var(--muted); }
.an-err { margin:0 4px; font-size:12px; color:var(--loss); }
.an-grid { margin:0 4px; display:grid; grid-template-columns:minmax(0,1.7fr) minmax(0,1fr); gap:4px; align-items:start; }
.an-grid .panel { margin:0; }
.an-col { display:flex; flex-direction:column; gap:4px; min-width:0; }
.an-head { display:flex; align-items:center; gap:10px; flex-wrap:wrap; padding:6px 12px; min-height:41px; border-bottom:1px solid var(--rule); }
.an-head .label { color:var(--ink); font-size:13px; letter-spacing:0; text-transform:none; }
svg.an-chart { width:100%; height:240px; display:block; padding:8px 12px; }
svg.an-hist { width:100%; height:170px; display:block; padding:8px 12px; }
svg.an-chart text, svg.an-hist text { fill:var(--muted); font-family:var(--font-mono); font-size:10px; }
.scroll { overflow-x:auto; }
.scroll.feed { max-height:340px; overflow-y:auto; }
.scroll thead th { position:sticky; top:0; background:var(--surface); }
.tt button.addr { border:none; padding:0; background:transparent; font-family:var(--font-mono); font-size:12px; color:var(--ink); cursor:pointer; }
.tt button.addr:hover { color:var(--calm); text-decoration:underline; }
.badge { margin-left:6px; font-size:10px; font-weight:600; text-transform:uppercase; letter-spacing:.06em; color:var(--muted);
  border:1px solid var(--rule); border-radius:999px; padding:0 6px; }
.live-dot { font-size:11px; font-weight:600; }
.analytics th:nth-child(-n+3), .analytics td:nth-child(-n+3) { text-align:left; }
.analytics .empty { padding:16px 12px; }

/* docs */
.docs { margin:0 4px; display:grid; grid-template-columns:220px minmax(0,1fr); gap:4px; align-items:start; }
.docs .panel { margin:0; }
.docs-toc { position:sticky; top:4px; display:flex; flex-direction:column; gap:2px; padding:12px; }
.docs-toc .label { margin-bottom:6px; }
.tt .docs-toc a { color:var(--muted); text-decoration:none; padding:4px 10px; border-left:2px solid transparent; }
.tt .docs-toc a:hover { color:var(--ink); }
.tt .docs-toc a.on { color:var(--ink); font-weight:600; border-left-color:var(--calm); }
.docs-body { padding:24px 40px 48px; font-size:15px; line-height:22px; }
.docs-body > * { max-width:760px; }
.docs-body h1 { font-family:var(--font-display); font-size:40px; line-height:40px; font-weight:700; margin:0 0 6px; }
.docs-section { padding-top:24px; scroll-margin-top:8px; }
.docs-section h2 { font-size:20px; line-height:26px; font-weight:600; margin:0 0 10px; padding-bottom:8px; border-bottom:1px solid var(--rule); }
.docs-section p, .docs-section ul { margin:0 0 12px; }
.docs-section ul { padding-left:20px; }
.docs-section strong { font-weight:600; }
.formula { display:flex; flex-wrap:wrap; gap:4px 16px; align-items:baseline; margin:12px 0; padding:10px 14px; border-radius:2px;
  background:var(--ground); border-left:2px solid var(--calm); }
.formula .num { font-size:13px; } .formula .muted { font-size:12px; }
.note { margin:12px 0; padding:10px 12px; border-radius:6px; border:1px solid var(--wild); color:var(--ink); font-size:14px; line-height:20px; }
.tt .docs-table { margin:8px 0 14px; font-size:13px; }
.tt .docs-table th, .tt .docs-table td { text-align:left; padding:8px 12px 8px 0; vertical-align:top; white-space:normal; }
.tt .docs-table td { font-family:var(--font-sans); font-size:13px; }

/* phone */
@media (max-width:759px) {
  .tt { padding-bottom:60px; }
  .nav { padding:0 12px; } .navlinks, .nav .theme { display:none; }
  .tabbar { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); position:fixed; left:0; right:0; bottom:0; z-index:10;
    background:var(--surface); border-top:1px solid var(--rule); }
  .tt .tabbar a { height:52px; display:flex; align-items:center; justify-content:center; color:var(--muted); text-decoration:none; }
  .tt .tabbar a.on { color:var(--ink); font-weight:600; box-shadow:inset 0 2px 0 var(--calm); }
  .mbar { gap:6px 16px; } .mbar .opt { display:none; } .mark-readout { font-size:40px; line-height:40px; }
  .main { flex-direction:column; }
  .main > .panel { width:100%; flex:none; }
  .quotes-panel, .ticket-panel { max-width:none; }
  .ticket-panel { order:-1; }
  .kpis { grid-template-columns:repeat(2,minmax(0,1fr)); }
  .an-grid { grid-template-columns:minmax(0,1fr); }
  svg.an-chart { height:180px; }
  .docs { grid-template-columns:1fr; }
  .docs-toc { position:static; flex-direction:row; flex-wrap:wrap; gap:4px 12px; }
  .tt .docs-toc a { border-left:none; padding:2px 0; }
  .docs-body { padding:16px 16px 32px; }
}

/* flow of funds (where trader PnL went) */
.fof-sentence { margin:0; padding:10px 12px 6px; font-size:13px; line-height:18px; color:var(--ink); }
.fof-bar { display:flex; height:10px; margin:4px 12px; border-radius:2px; overflow:hidden; background:var(--rule); }
.fof-seg { display:block; height:100%; min-width:2px; }
.fof-seg.fees, .fof-sw.fees { background:var(--loss); }
.fof-seg.makers, .fof-sw.makers { background:var(--loss); opacity:.45; }
.fof-legend { display:flex; flex-wrap:wrap; gap:4px 16px; padding:2px 12px 6px; font-size:12px; align-items:center; }
.fof-legend > span { display:inline-flex; align-items:center; gap:6px; }
.fof-sw { display:inline-block; width:10px; height:10px; border-radius:2px; }
.tt .fof-table { width:100%; border-collapse:collapse; }
.tt .fof-table th { border-top:1px solid var(--rule); vertical-align:top; text-align:left; font-weight:400; letter-spacing:0; text-transform:none; white-space:normal; }
.tt .fof-table th .lbl { display:block; }
.fof-note { display:block; font-size:11px; line-height:14px; }
.tt .fof-table td { text-align:right; vertical-align:top; white-space:nowrap; }
.tt .fof-table tr.fof-total th, .tt .fof-table tr.fof-total td { border-top:2px solid var(--muted); }
.tt .fof-table tr.fof-total .lbl { color:var(--ink); }
.tt .fof-cp tr:first-child th { padding-top:14px; }
"#;
