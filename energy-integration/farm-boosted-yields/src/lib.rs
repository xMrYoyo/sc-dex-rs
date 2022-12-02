#![no_std]

elrond_wasm::imports!();
elrond_wasm::derive_imports!();

use core::cmp;

use common_types::PaymentsVec;
use week_timekeeping::Week;
use weekly_rewards_splitting::{
    base_impl::WeeklyRewardsSplittingTraitsModule, USER_MAX_CLAIM_WEEKS,
};

const MAX_PERCENT: u64 = 10_000;

pub struct SplitReward<M: ManagedTypeApi> {
    pub base_farm: BigUint<M>,
    pub boosted_farm: BigUint<M>,
}

impl<M: ManagedTypeApi> SplitReward<M> {
    pub fn new(base_farm: BigUint<M>, boosted_farm: BigUint<M>) -> Self {
        SplitReward {
            base_farm,
            boosted_farm,
        }
    }
}

#[derive(TypeAbi, TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, PartialEq, Debug)]
pub struct BoostedYieldsFactors<M: ManagedTypeApi> {
    pub max_rewards_factor: BigUint<M>,
    pub user_rewards_energy_const: BigUint<M>,
    pub user_rewards_farm_const: BigUint<M>,
    pub min_energy_amount: BigUint<M>,
    pub min_farm_amount: BigUint<M>,
}

#[elrond_wasm::module]
pub trait FarmBoostedYieldsModule:
    config::ConfigModule
    + week_timekeeping::WeekTimekeepingModule
    + pausable::PausableModule
    + permissions_module::PermissionsModule
    + weekly_rewards_splitting::WeeklyRewardsSplittingModule
    + weekly_rewards_splitting::events::WeeklyRewardsSplittingEventsModule
    + weekly_rewards_splitting::global_info::WeeklyRewardsGlobalInfo
    + weekly_rewards_splitting::locked_token_buckets::WeeklyRewardsLockedTokenBucketsModule
    + weekly_rewards_splitting::update_claim_progress_energy::UpdateClaimProgressEnergyModule
    + energy_query::EnergyQueryModule
{
    #[endpoint(setBoostedYieldsRewardsPercentage)]
    fn set_boosted_yields_rewards_percentage(&self, percentage: u64) {
        self.require_caller_has_admin_permissions();
        require!(percentage <= MAX_PERCENT, "Invalid percentage");

        self.boosted_yields_rewards_percentage().set(percentage);
    }

    #[endpoint(setBoostedYieldsFactors)]
    fn set_boosted_yields_factors(
        &self,
        max_rewards_factor: BigUint,
        user_rewards_energy_const: BigUint,
        user_rewards_farm_const: BigUint,
        min_energy_amount: BigUint,
        min_farm_amount: BigUint,
    ) {
        self.require_caller_has_admin_permissions();
        require!(
            max_rewards_factor > 0
                && user_rewards_energy_const > 0
                && user_rewards_farm_const > 0
                && min_energy_amount > 0
                && min_farm_amount > 0,
            "Values must be greater than 0"
        );

        let factors = BoostedYieldsFactors {
            max_rewards_factor,
            user_rewards_energy_const,
            user_rewards_farm_const,
            min_energy_amount,
            min_farm_amount,
        };
        self.boosted_yields_factors().set(factors);
    }

    #[endpoint(collectUndistributedBoostedRewards)]
    fn collect_undistributed_boosted_rewards(&self) {
        self.require_caller_has_admin_permissions();

        let collect_rewards_offset = USER_MAX_CLAIM_WEEKS + 1usize;
        let current_week = self.get_current_week();
        require!(
            current_week > collect_rewards_offset,
            "Current week must be higher than the week offset"
        );

        let last_collect_week_mapper = self.last_undistributed_boosted_rewards_collect_week();
        let first_collect_week = last_collect_week_mapper.get() + 1;
        let last_collect_week = current_week - collect_rewards_offset;
        if first_collect_week > last_collect_week {
            return;
        }

        for week in first_collect_week..=last_collect_week {
            let rewards_to_distribute_mapper = self.remaining_boosted_rewards_to_distribute(week);
            let rewards_to_distribute = rewards_to_distribute_mapper.get();
            self.undistributed_boosted_rewards()
                .update(|total_amount| *total_amount += rewards_to_distribute);
            rewards_to_distribute_mapper.clear();
        }

        last_collect_week_mapper.set(last_collect_week);
    }

    fn take_reward_slice(&self, full_reward: BigUint) -> SplitReward<Self::Api> {
        let percentage = self.boosted_yields_rewards_percentage().get();
        if percentage == 0 {
            return SplitReward::new(full_reward, BigUint::zero());
        }

        let boosted_yields_cut = &full_reward * percentage / MAX_PERCENT;
        let base_farm_amount = if boosted_yields_cut > 0 {
            let current_week = self.get_current_week();
            self.accumulated_rewards_for_week(current_week)
                .update(|accumulated_rewards| {
                    *accumulated_rewards += &boosted_yields_cut;
                });

            &full_reward - &boosted_yields_cut
        } else {
            full_reward
        };

        SplitReward::new(base_farm_amount, boosted_yields_cut)
    }

    fn claim_boosted_yields_rewards(
        &self,
        user: &ManagedAddress,
        farm_token_amount: BigUint,
    ) -> BigUint {
        let wrapper = FarmBoostedYieldsWrapper::new(farm_token_amount);
        let rewards = self.claim_multi(&wrapper, user);

        let mut total = BigUint::zero();
        for rew in &rewards {
            total += rew.amount;
        }

        total
    }

    #[view(getBoostedYieldsRewardsPercenatage)]
    #[storage_mapper("boostedYieldsRewardsPercentage")]
    fn boosted_yields_rewards_percentage(&self) -> SingleValueMapper<u64>;

    #[storage_mapper("accumulatedRewardsForWeek")]
    fn accumulated_rewards_for_week(&self, week: Week) -> SingleValueMapper<BigUint>;

    #[view(getFarmSupplyForWeek)]
    #[storage_mapper("farmSupplyForWeek")]
    fn farm_supply_for_week(&self, week: Week) -> SingleValueMapper<BigUint>;

    #[view(getRemainingBoostedRewardsToDistribute)]
    #[storage_mapper("remainingBoostedRewardsToDistribute")]
    fn remaining_boosted_rewards_to_distribute(&self, week: Week) -> SingleValueMapper<BigUint>;

    #[storage_mapper("lastUndistributedBoostedRewardsCollectWeek")]
    fn last_undistributed_boosted_rewards_collect_week(&self) -> SingleValueMapper<Week>;

    #[view(getUndistributedBoostedRewards)]
    #[storage_mapper("undistributedBoostedRewards")]
    fn undistributed_boosted_rewards(&self) -> SingleValueMapper<BigUint>;

    #[view(getBoostedYieldsFactors)]
    #[storage_mapper("boostedYieldsFactors")]
    fn boosted_yields_factors(&self) -> SingleValueMapper<BoostedYieldsFactors<Self::Api>>;
}

pub struct FarmBoostedYieldsWrapper<T: FarmBoostedYieldsModule> {
    pub user_farm_amount: BigUint<<T as ContractBase>::Api>,
}

impl<T: FarmBoostedYieldsModule> FarmBoostedYieldsWrapper<T> {
    pub fn new(user_farm_amount: BigUint<<T as ContractBase>::Api>) -> FarmBoostedYieldsWrapper<T> {
        FarmBoostedYieldsWrapper { user_farm_amount }
    }
}

impl<T> WeeklyRewardsSplittingTraitsModule for FarmBoostedYieldsWrapper<T>
where
    T: FarmBoostedYieldsModule,
{
    type WeeklyRewardsSplittingMod = T;

    fn collect_rewards_for_week(
        &self,
        module: &Self::WeeklyRewardsSplittingMod,
        week: Week,
    ) -> PaymentsVec<<Self::WeeklyRewardsSplittingMod as ContractBase>::Api> {
        let reward_token_id = module.reward_token_id().get();
        let rewards_mapper = module.accumulated_rewards_for_week(week);
        let total_rewards = rewards_mapper.get();
        rewards_mapper.clear();

        module
            .remaining_boosted_rewards_to_distribute(week)
            .set(&total_rewards);

        ManagedVec::from_single_item(EsdtTokenPayment::new(reward_token_id, 0, total_rewards))
    }

    fn get_user_rewards_for_week(
        &self,
        module: &Self::WeeklyRewardsSplittingMod,
        week: Week,
        energy_amount: &BigUint<<Self::WeeklyRewardsSplittingMod as ContractBase>::Api>,
        total_energy: &BigUint<<Self::WeeklyRewardsSplittingMod as ContractBase>::Api>,
    ) -> PaymentsVec<<Self::WeeklyRewardsSplittingMod as ContractBase>::Api> {
        let mut user_rewards = ManagedVec::new();
        if module.boosted_yields_factors().is_empty() {
            return user_rewards;
        }

        let farm_supply_for_week = module.farm_supply_for_week(week).get();
        if total_energy == &0 || farm_supply_for_week == 0 {
            return user_rewards;
        }

        let factors = module.boosted_yields_factors().get();
        if energy_amount < &factors.min_energy_amount
            || self.user_farm_amount < factors.min_farm_amount
        {
            return user_rewards;
        }

        let total_rewards = self.collect_and_get_rewards_for_week(module, week);
        if total_rewards.is_empty() {
            return user_rewards;
        }

        // always no entries or 1 entry, but the trait uses a Vec
        if total_rewards.len() != 1 {
            <<Self::WeeklyRewardsSplittingMod as ContractBase>::Api>::error_api_impl()
                .signal_error(b"Invalid boosted yields rewards");
        }

        let weekly_reward = total_rewards.get(0);
        if weekly_reward.amount == 0 {
            return user_rewards;
        }

        let max_rewards =
            &factors.max_rewards_factor * &weekly_reward.amount * &self.user_farm_amount
                / &farm_supply_for_week;

        // computed user rewards = total_boosted_rewards *
        // (energy_const * user_energy / total_energy + farm_const * user_farm / total_farm) /
        // (energy_const + farm_const)
        let boosted_rewards_by_energy =
            &weekly_reward.amount * &factors.user_rewards_energy_const * energy_amount
                / total_energy;
        let boosted_rewards_by_tokens =
            &weekly_reward.amount * &factors.user_rewards_farm_const * &self.user_farm_amount
                / &farm_supply_for_week;
        let constants_base = &factors.user_rewards_energy_const + &factors.user_rewards_farm_const;
        let boosted_reward_amount =
            (boosted_rewards_by_energy + boosted_rewards_by_tokens) / constants_base;

        // min between base rewards per week and computed rewards
        let user_reward = cmp::min(max_rewards, boosted_reward_amount);
        if user_reward > 0 {
            module
                .remaining_boosted_rewards_to_distribute(week)
                .update(|amount| *amount -= &user_reward);

            user_rewards.push(EsdtTokenPayment::new(
                weekly_reward.token_identifier,
                0,
                user_reward,
            ));
        }

        user_rewards
    }
}
