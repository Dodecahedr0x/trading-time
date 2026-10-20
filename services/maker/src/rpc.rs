use crate::chain::{Action, Chain};
use crate::slot::{PerfSample, SAMPLE_LIMIT};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use tradingtime_client::{CurveQuote, ix, rpc::Ctx};
use tradingtime_interface::{Market, Operator, TOKEN_PROGRAM, Trader, USDC_MINT};

const ATA_PROGRAM: Pubkey = Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub fn usdc_ata(owner: &Pubkey) -> Pubkey {
    let (mint, token) = (Pubkey::new_from_array(USDC_MINT), Pubkey::new_from_array(TOKEN_PROGRAM));
    Pubkey::find_program_address(&[owner.as_ref(), token.as_ref(), mint.as_ref()], &ATA_PROGRAM).0
}

pub struct RpcChain {
    ctx: Ctx,
    owner: Pubkey,
    owner_usdc: Pubkey,
}

impl RpcChain {
    pub fn new(url: &str, payer: Keypair) -> Self {
        let owner = payer.pubkey();
        RpcChain { ctx: Ctx::new(url, payer), owner, owner_usdc: usdc_ata(&owner) }
    }

    pub fn owner(&self) -> Pubkey {
        self.owner
    }

    fn ix(&self, a: &Action) -> solana_instruction::Instruction {
        let o = &self.owner;
        match *a {
            Action::CreateTrader => ix::create_trader(o),
            Action::Deposit(amount) => ix::deposit(o, &self.owner_usdc, amount),
            Action::RegisterOperator => ix::register_operator(o, &self.owner_usdc),
            Action::UpdateCurve(q) => ix::update_curve(
                o,
                CurveQuote {
                    fair_ln: q.fair_ln,
                    half_spread: q.half_spread,
                    impact_per_usdc: q.impact_per_usdc,
                    inventory_limit: q.inventory_limit,
                },
            ),
            Action::RequestUnbond => ix::request_unbond(o),
        }
    }
}

impl Chain for RpcChain {
    async fn market(&self) -> anyhow::Result<Market> {
        self.ctx.market().await
    }

    async fn trader(&self) -> anyhow::Result<Option<Trader>> {
        self.ctx.trader(&self.owner).await
    }

    async fn operator(&self) -> anyhow::Result<Option<Operator>> {
        let owner = self.owner.to_bytes();
        Ok(self.ctx.operators().await?.into_iter().map(|(_, op)| op).find(|op| op.owner == owner))
    }

    async fn send(&self, actions: &[Action]) -> anyhow::Result<()> {
        let ixs: Vec<_> = actions.iter().map(|a| self.ix(a)).collect();
        self.ctx.send(&ixs, &[]).await?;
        Ok(())
    }

    async fn perf_samples(&self) -> anyhow::Result<Vec<PerfSample>> {
        let samples = self.ctx.client.get_recent_performance_samples(Some(SAMPLE_LIMIT)).await?;
        Ok(samples
            .into_iter()
            .map(|s| PerfSample { num_slots: s.num_slots, period_secs: u64::from(s.sample_period_secs) })
            .collect())
    }
}
