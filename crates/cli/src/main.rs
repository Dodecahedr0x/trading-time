//! Headless backtests.
//!
//! cli [--feed gbm|regime|jump|csv] [--path file.csv] [--seed N] [--steps N] [--cap LOG_PER_MIN] [--worst-of 0|1] [--iv-weight W] [--iv-noise SD] [--iv-premium P] [--log 0|1] [--max-lev L] [--max-skew FRAC] [--lp USD] [--adl 0|1]
//!     [--noise N] [--informed N] [--momentum N] [--chaser N] [--fast N] [--export out.csv]
//!     [--venue pool|book] [--mm N] [--mm-inv USD] [--mm-hl SECS]   (book venue uses --max-lev and the agent counts; pool flags are ignored)

use timeperp::book_sim::{BookSim, BookSimConfig};
use timeperp::engine::{Sim, SimConfig};
use timeperp::feed::{CsvFeed, Model, PriceFeed, SynthFeed, to_csv};
use timeperp::oracle::sigma_per_sec;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    const FLAGS: [&str; 24] = [
        "--feed",
        "--path",
        "--seed",
        "--steps",
        "--cap",
        "--worst-of",
        "--iv-weight",
        "--iv-noise",
        "--iv-premium",
        "--log",
        "--max-lev",
        "--max-skew",
        "--lp",
        "--adl",
        "--noise",
        "--informed",
        "--momentum",
        "--chaser",
        "--fast",
        "--venue",
        "--mm",
        "--mm-inv",
        "--mm-hl",
        "--export",
    ];
    if let Some(bad) = args.iter().step_by(2).find(|a| !FLAGS.contains(&a.as_str())) {
        return Err(format!("unknown flag {bad:?}; expected one of {FLAGS:?}"));
    }
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str);
    let num =
        |name: &str, default: u64| arg(name).map_or(Ok(default), |v| v.parse().map_err(|e| format!("{name}: {e}")));
    let seed = num("--seed", 1)?;
    let steps = num("--steps", 100_000)?;

    let mut feed: Box<dyn PriceFeed> = match arg("--feed").unwrap_or("regime") {
        "gbm" => Box::new(SynthFeed::new(Model::Gbm { sigma: sigma_per_sec(0.8) }, seed, 150.0, 60.0)),
        "regime" => Box::new(SynthFeed::new(
            Model::Regime { sigmas: vec![sigma_per_sec(0.5), sigma_per_sec(1.5)], switch_prob: 1.0 / 1_440.0 },
            seed,
            150.0,
            60.0,
        )),
        "jump" => Box::new(SynthFeed::new(
            Model::Jump { sigma: sigma_per_sec(0.6), lambda: 1.0 / 86_400.0, jump_sigma: 0.04 },
            seed,
            150.0,
            60.0,
        )),
        "csv" => Box::new(CsvFeed::load(arg("--path").ok_or("--feed csv needs --path")?)?),
        other => return Err(format!("unknown feed {other}")),
    };

    if let Some(out) = arg("--export") {
        let bars: Vec<_> = (0..steps).map_while(|_| feed.next_bar()).collect();
        std::fs::write(out, to_csv(&bars)).map_err(|e| e.to_string())?;
        println!("wrote {} bars to {out}", bars.len());
        return Ok(());
    }

    let float = |name: &str| arg(name).map(|v| v.parse::<f64>().map_err(|e| format!("{name}: {e}"))).transpose();
    if arg("--venue") == Some("book") {
        let mut cfg = BookSimConfig { seed, ..Default::default() };
        if let Some(lev) = float("--max-lev")? {
            cfg.book.max_leverage = lev;
        }
        cfg.mm = num("--mm", cfg.mm as u64)? as usize;
        if let Some(inv) = float("--mm-inv")? {
            cfg.mm_inventory_limit = inv;
        }
        if let Some(hl) = float("--mm-hl")? {
            cfg.mm_half_life = hl;
        }
        cfg.noise = num("--noise", cfg.noise as u64)? as usize;
        cfg.informed = num("--informed", cfg.informed as u64)? as usize;
        cfg.momentum = num("--momentum", cfg.momentum as u64)? as usize;
        cfg.fast = num("--fast", cfg.fast as u64)? as usize;
        let mut sim = BookSim::new(cfg, feed)?;
        for _ in 0..steps {
            if !sim.step() {
                break;
            }
        }
        print!("{}", sim.summary());
        return Ok(());
    }
    let max_move_per_min = float("--cap")?;
    let mut cfg = SimConfig { seed, max_move_per_min, ..Default::default() };
    cfg.iv_weight = float("--iv-weight")?.unwrap_or(0.0);
    if let Some(noise) = float("--iv-noise")? {
        cfg.implied.noise = noise;
    }
    if let Some(premium) = float("--iv-premium")? {
        cfg.implied.premium = premium;
    }
    let worst_of_pricing = num("--worst-of", 1)? != 0;
    cfg.worst_of_pricing = worst_of_pricing;
    cfg.pool.log_quote = num("--log", 0)? != 0;
    cfg.pool.adl = num("--adl", 0)? != 0;
    if let Some(lev) = float("--max-lev")? {
        cfg.pool.max_leverage = lev;
    }
    if let Some(skew) = float("--max-skew")? {
        cfg.pool.max_net_skew = skew;
    }
    if let Some(lp) = float("--lp")? {
        cfg.lp_liquidity = lp;
    }
    let mix = &mut cfg.agents;
    mix.noise = num("--noise", mix.noise as u64)? as usize;
    mix.informed = num("--informed", mix.informed as u64)? as usize;
    mix.momentum = num("--momentum", mix.momentum as u64)? as usize;
    mix.chaser = num("--chaser", mix.chaser as u64)? as usize;
    mix.fast = num("--fast", mix.fast as u64)? as usize;
    let mut sim = Sim::new(cfg, feed)?;
    for _ in 0..steps {
        if !sim.step() {
            break;
        }
    }
    print!("{}", sim.summary());
    Ok(())
}
