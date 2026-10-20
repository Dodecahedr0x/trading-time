pub const FONTS: &str = "https://fonts.googleapis.com/css2?family=Big+Shoulders+Display:wght@600;700&family=Hanken+Grotesk:wght@400;600&family=Martian+Mono:wght@400;500&display=swap";

pub const CSS: &str = r#"
.tt { --ground:#0e1120; --surface:#171b2e; --ink:#e8eaf3; --muted:#9ca3bf; --rule:#3a4060;
  --calm:#7aa2ff; --on-calm:#0e1120; --wild:#ff8f4f; --on-wild:#0e1120; --gain:#45c29b; --loss:#ff7a6e; }
.tt.light { --ground:#f1f2f6; --surface:#fbfbfd; --ink:#161a2e; --muted:#545b78; --rule:#c9cdda;
  --calm:#1f5fd1; --on-calm:#ffffff; --wild:#b8460f; --on-wild:#ffffff; --gain:#0b7354; --loss:#c0352b; }
html, body { margin:0; background:#0e1120; }
.tt { background:var(--ground); color:var(--ink); font:400 13px/1.4 "Hanken Grotesk", system-ui, sans-serif;
  min-height:100vh; display:flex; flex-direction:column; gap:4px; padding:4px; box-sizing:border-box; }
.tt * { box-sizing:border-box; }
.panel { background:var(--surface); border-radius:2px; padding:10px 12px; min-width:0; }
.label { font-size:11px; font-weight:600; text-transform:uppercase; letter-spacing:.06em; color:var(--muted); }
.num { font-family:"Martian Mono", ui-monospace, monospace; font-variant-numeric:tabular-nums; }
.calm { color:var(--calm); } .wild { color:var(--wild); } .gain { color:var(--gain); } .loss { color:var(--loss); } .muted { color:var(--muted); }
button { font:inherit; color:var(--ink); background:transparent; border:1px solid var(--muted); border-radius:2px; padding:6px 10px; cursor:pointer; }
button:focus-visible, input:focus-visible { outline:2px solid var(--calm); outline-offset:1px; }
input { font-family:"Martian Mono", monospace; font-size:13px; color:var(--ink); background:var(--ground); border:1px solid var(--muted);
  border-radius:2px; padding:8px; width:100%; }
.nav { display:flex; align-items:center; gap:16px; }
.wordmark { font-family:"Big Shoulders Display", sans-serif; font-weight:700; font-size:22px; letter-spacing:.02em; }
.nav .grow { flex:1; }
.nav .market-name { font-weight:600; }
.mbar { display:flex; align-items:flex-end; gap:24px; flex-wrap:wrap; }
.mbar .stat { display:flex; flex-direction:column; gap:2px; }
.mark-readout { font-family:"Big Shoulders Display", sans-serif; font-weight:600; font-size:30px; line-height:1; }
.explain { color:var(--muted); }
.main { display:grid; grid-template-columns:minmax(0,1fr) 260px 320px; gap:4px; }
.chart-head { display:flex; justify-content:space-between; align-items:center; margin-bottom:6px; }
.seg { display:flex; gap:0; }
.seg button { border-radius:0; padding:4px 10px; }
.seg button.on { background:var(--ink); color:var(--ground); border-color:var(--ink); }
svg.chart { width:100%; height:360px; display:block; }
svg.chart text { fill:var(--muted); font-family:"Martian Mono", monospace; font-size:10px; }
.ladder { display:flex; flex-direction:column; font-family:"Martian Mono", monospace; font-size:12px; }
.lrow { position:relative; display:grid; grid-template-columns:1fr 1fr 1fr; padding:2px 4px; cursor:pointer; }
.lrow span { position:relative; text-align:right; } .lrow span:first-child { text-align:left; }
.lrow .bar { position:absolute; top:0; bottom:0; right:0; opacity:.16; }
.lrow.ask .bar { background:var(--calm); } .lrow.bid .bar { background:var(--wild); }
.lmark { padding:6px 4px; border-top:1px solid var(--rule); border-bottom:1px solid var(--rule); display:flex; justify-content:space-between; }
.lmark .v { font-family:"Big Shoulders Display", sans-serif; font-size:18px; font-weight:600; }
.ticket { display:flex; flex-direction:column; gap:10px; }
.tabs { display:flex; gap:16px; border-bottom:1px solid var(--rule); }
.tabs button { border:none; padding:6px 0; color:var(--muted); font-weight:600; border-bottom:2px solid transparent; border-radius:0; }
.tabs button.on { color:var(--ink); border-bottom-color:var(--ink); }
.sides { display:grid; grid-template-columns:1fr 1fr; gap:4px; }
.sides button { padding:9px; font-weight:600; }
.sides button.long.on { background:var(--calm); color:var(--on-calm); border-color:var(--calm); }
.sides button.short.on { background:var(--wild); color:var(--on-wild); border-color:var(--wild); }
.presets { display:grid; grid-template-columns:repeat(4,1fr); gap:4px; } .presets button { padding:4px; font-size:12px; }
.buying { border:1px solid var(--rule); border-radius:2px; padding:10px; display:flex; flex-direction:column; gap:6px; }
.buying .head { font-weight:600; }
.line { display:flex; justify-content:space-between; gap:8px; }
.line .v { font-family:"Martian Mono", monospace; }
.buy { padding:12px; font-weight:600; font-size:14px; border:none; }
.buy.long { background:var(--calm); color:var(--on-calm); } .buy.short { background:var(--wild); color:var(--on-wild); }
.buy:disabled { opacity:.5; cursor:default; }
.txs { font-size:12px; }
table { width:100%; border-collapse:collapse; }
th { text-align:left; font-size:11px; font-weight:600; text-transform:uppercase; letter-spacing:.06em; color:var(--muted); padding:6px 8px; }
td { padding:6px 8px; border-top:1px solid var(--rule); font-family:"Martian Mono", monospace; font-size:12px; }
td.side { font-family:"Hanken Grotesk", sans-serif; font-weight:600; }
.empty { color:var(--muted); padding:12px 8px; }
.connect { background:var(--calm); color:var(--on-calm); border-color:var(--calm); font-weight:600; }
.navlinks { display:flex; gap:14px; }
.navlink { color:var(--muted); text-decoration:none; font-weight:600; padding:4px 0; border-bottom:2px solid transparent; }
.navlink:hover { color:var(--ink); }
.navlink.on { color:var(--ink); border-bottom-color:var(--calm); }
.navlink:focus-visible, select:focus-visible { outline:2px solid var(--calm); outline-offset:2px; }
select { font:inherit; font-size:12px; color:var(--ink); background:var(--ground); border:1px solid var(--muted); border-radius:2px; padding:4px 6px; }
.analytics { display:flex; flex-direction:column; gap:4px; }
.kpis { display:grid; grid-template-columns:repeat(auto-fit, minmax(150px, 1fr)); gap:4px; }
.kpi { display:flex; flex-direction:column; gap:4px; }
.kpi-v { font-size:20px; font-weight:500; line-height:1.1; }
.kpi-sub { font-size:11px; }
.an-err { font-size:12px; }
.an-grid { display:grid; grid-template-columns:minmax(0,1.7fr) minmax(0,1fr); gap:4px; align-items:start; }
.an-col { display:flex; flex-direction:column; gap:4px; min-width:0; }
.an-head { display:flex; align-items:center; gap:10px; flex-wrap:wrap; margin-bottom:6px; }
.an-head .grow { flex:1; }
.toggle { padding:4px 10px; font-size:12px; }
.toggle.on { background:var(--ink); color:var(--ground); border-color:var(--ink); }
svg.an-chart { width:100%; height:240px; display:block; }
svg.an-hist { width:100%; height:170px; display:block; }
svg.an-chart text, svg.an-hist text { fill:var(--muted); font-family:"Martian Mono", monospace; font-size:10px; }
.scroll { overflow-x:auto; }
.scroll.feed { max-height:320px; overflow-y:auto; }
.scroll thead th { position:sticky; top:0; background:var(--surface); }
.addr { border:none; padding:0; font-size:12px; color:var(--ink); }
.addr:hover { color:var(--calm); text-decoration:underline; }
.badge { margin-left:6px; font-size:10px; font-weight:600; text-transform:uppercase; letter-spacing:.06em; color:var(--muted);
  border:1px solid var(--rule); border-radius:2px; padding:0 4px; }
.live-dot { font-size:11px; font-weight:600; }
@media (max-width: 759px) {
  .an-grid { grid-template-columns:minmax(0,1fr); }
  .kpis { grid-template-columns:repeat(2, minmax(0,1fr)); }
  svg.an-chart { height:180px; }
  .main { grid-template-columns:minmax(0,1fr); }
  .mbar { gap:12px; } .mbar .opt { display:none; }
  svg.chart { height:220px; }
  .nav .market-name, .nav .theme { display:none; }
  .ticket-panel { order:-1; }
  .acct { overflow-x:auto; }
}
"#;
