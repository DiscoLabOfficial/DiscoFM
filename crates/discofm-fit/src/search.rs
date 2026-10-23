use crate::{
    FitCandidate, FitContext, FitError, FitInput, FitMode, FitOutcome, FitProgress, FitStage,
    FitStatus, HistoryPoint, LossBreakdown, MAX_HISTORY, OperatorField, Parameter,
    loss::{Objective, check},
    parameter::{key, searchable},
};
use discofm_chip::Instrument;
use std::{collections::BTreeMap, time::Instant};

/// Operator (MUL, TL) pair of a structured timbre seed.
type Ratio = (u16, u16);
/// SplitMix64, fixed algorithm and seed; no platform/thread RNG dependency.
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / 9007199254740992.0)
    }
    fn index(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn donors(&mut self, size: usize, target: usize) -> [usize; 3] {
        let mut result = [target; 3];
        for i in 0..3 {
            loop {
                let index = self.index(size);
                if index != target && !result[..i].contains(&index) {
                    result[i] = index;
                    break;
                }
            }
        }
        result
    }
}
/// Convergence curve bounded to `MAX_HISTORY` points. When full, every other
/// interior point goes and the sampling stride doubles, so a long run keeps an
/// evenly spaced curve with each stage's first point; dropping the oldest point
/// instead left only the baseline and the last few percent of a large budget.
struct History {
    points: Vec<HistoryPoint>,
    stride: usize,
}
impl History {
    fn new() -> Self {
        Self {
            points: Vec::new(),
            stride: 8,
        }
    }
    fn record(&mut self, point: HistoryPoint) {
        if self
            .points
            .last()
            .is_none_or(|p| point.best < p.best || p.stage != point.stage)
            || point.evaluation.is_multiple_of(self.stride)
        {
            self.push(point);
        }
    }
    fn push(&mut self, point: HistoryPoint) {
        self.points.push(point);
        if self.points.len() <= MAX_HISTORY {
            return;
        }
        let last = self.points.len() - 1;
        let mut previous = None;
        let mut interior = 0_usize;
        let mut index = 0;
        self.points.retain(|p| {
            let boundary = previous != Some(p.stage);
            previous = Some(p.stage);
            let keep = if index == 0 || index == last || boundary {
                true
            } else {
                interior += 1;
                interior.is_multiple_of(2)
            };
            index += 1;
            keep
        });
        self.stride *= 2;
    }
}
#[derive(Clone)]
struct Member {
    base: Instrument,
    genes: Vec<f64>,
    loss: LossBreakdown,
    /// Differential weight and crossover rate this member was produced with.
    f: f64,
    cr: f64,
}
/// ymfm scores used only to choose among proposals; see `FitConfig::screening`.
type Screen<'a> = Box<dyn FnMut(&[Instrument]) -> Result<Vec<f64>, FitError> + 'a>;
struct Search<'a, E, P> {
    input: &'a FitInput,
    evaluate: E,
    screen: Option<Screen<'a>>,
    /// ymfm scores by register key; never ranked or recorded.
    screen_cache: BTreeMap<Vec<u16>, f64>,
    progress: P,
    cache: BTreeMap<Vec<u16>, LossBreakdown>,
    /// Scores of lookahead proposals not yet requested; never recorded or ranked.
    speculative: BTreeMap<Vec<u16>, LossBreakdown>,
    candidates: Vec<FitCandidate>,
    families: [Option<FitCandidate>; 16],
    history: History,
    initial_loss: Option<LossBreakdown>,
    evaluations: usize,
    renders: usize,
    generation: usize,
    current: f64,
    stage: FitStage,
    start: Instant,
    context: FitContext,
    anchors: Vec<crate::tracking::Peak>,
}
impl<E, P> Search<'_, E, P>
where
    E: FnMut(&[Instrument]) -> Result<Vec<LossBreakdown>, FitError>,
    P: FnMut(FitProgress),
{
    /// Best patch of an algorithm/topology family, once one has been scored.
    fn family(&self, index: usize) -> Option<Instrument> {
        self.families
            .get(index)?
            .as_ref()
            .map(|c| c.instrument.clone())
    }
    fn workers(&self) -> usize {
        if self.input.config.workers == 0 {
            crate::default_workers()
        } else {
            self.input.config.workers
        }
    }
    fn score_many(
        &mut self,
        patches: Vec<Instrument>,
        cancel: &impl Fn() -> bool,
    ) -> Result<Vec<LossBreakdown>, FitError> {
        self.score_ahead(patches, Vec::new(), cancel)
    }
    /// `lookahead` holds proposals the caller expects to make next if nothing
    /// improves. Workers left idle by a short batch score them speculatively.
    /// Decisions are unchanged: a stored score is used only when the search
    /// later proposes the identical patch, in place of rendering it again.
    fn score_ahead(
        &mut self,
        patches: Vec<Instrument>,
        lookahead: Vec<Instrument>,
        cancel: &impl Fn() -> bool,
    ) -> Result<Vec<LossBreakdown>, FitError> {
        check(cancel)?;
        let workers = self.workers();
        let chunks = patches.len().div_ceil(workers);
        let mut result = Vec::new();
        for (position, chunk) in patches.chunks(workers).enumerate() {
            check(cancel)?;
            let mut missing = Vec::new();
            let mut identities = std::collections::BTreeSet::new();
            let mut pending = BTreeMap::new();
            for patch in chunk {
                let identity = key(patch);
                if !self.cache.contains_key(&identity) && identities.insert(identity.clone()) {
                    match self.speculative.remove(&identity) {
                        Some(loss) => {
                            pending.insert(identity, loss);
                        }
                        None => missing.push(patch.clone()),
                    }
                }
            }
            let wanted = missing.len();
            // Every busy thread slows each render (shared clocks, SMT), so a
            // discarded guess delays the wanted scores. At the default budget,
            // half the workers measured faster than no lookahead or a full batch.
            let limit = workers.div_ceil(2).max(wanted);
            if wanted > 0 && position + 1 == chunks {
                for patch in &lookahead {
                    if missing.len() >= limit {
                        break;
                    }
                    let identity = key(patch);
                    if !self.cache.contains_key(&identity)
                        && !self.speculative.contains_key(&identity)
                        && identities.insert(identity)
                    {
                        missing.push(patch.clone());
                    }
                }
            }
            let mut partial = false;
            if wanted > 0 {
                let losses = (self.evaluate)(&missing)?;
                partial = losses.len() < wanted;
                if losses.len() > missing.len() || (partial && !cancel()) {
                    return Err(FitError::Settings("batch score count mismatch"));
                }
                if self.speculative.len() > 4096 {
                    self.speculative.clear();
                }
                for (index, (patch, loss)) in missing.into_iter().zip(losses).enumerate() {
                    loss.validate(&self.input.config.weights)?;
                    if index < wanted {
                        pending.insert(key(&patch), loss);
                    } else {
                        self.speculative.insert(key(&patch), loss);
                    }
                }
            }
            let completed_at_cancel = partial || cancel();
            for patch in chunk {
                if !completed_at_cancel {
                    check(cancel)?;
                }
                let identity = key(patch);
                if partial
                    && !self.cache.contains_key(&identity)
                    && !pending.contains_key(&identity)
                {
                    break;
                }
                let loss = if let Some(loss) = self.cache.get(&identity) {
                    *loss
                } else {
                    let loss = pending[&identity];
                    self.cache.insert(identity, loss);
                    self.renders += 1;
                    loss
                };
                self.record(patch.clone(), loss);
                result.push(loss);
            }
            if completed_at_cancel {
                return Err(FitError::Cancelled);
            }
        }
        Ok(result)
    }
    fn record(&mut self, patch: Instrument, loss: LossBreakdown) {
        self.evaluations += 1;
        self.current = loss.total;
        self.initial_loss.get_or_insert(loss);
        let candidate = FitCandidate {
            evaluation: self.evaluations,
            stage: self.stage,
            instrument: patch.clone(),
            loss,
        };
        let family = &mut self.families[usize::from(patch.channel.algorithm.get())
            + usize::from(patch.channel3_special.is_some()) * 8];
        if family
            .as_ref()
            .is_none_or(|old| loss.total < old.loss.total)
        {
            *family = Some(candidate.clone());
        }
        if !self.candidates.iter().any(|c| c.instrument == patch) {
            self.candidates.push(candidate);
            self.candidates.sort_by(|a, b| {
                a.loss
                    .total
                    .total_cmp(&b.loss.total)
                    .then(a.evaluation.cmp(&b.evaluation))
            });
            self.candidates.truncate(self.input.config.candidates);
        }
        self.history.record(HistoryPoint {
            evaluation: self.evaluations,
            stage: self.stage,
            best: self.candidates[0].loss.total,
            current: loss.total,
        });
        (self.progress)(FitProgress {
            stage: self.stage,
            evaluations: self.evaluations,
            budget: self.input.config.evaluations,
            renders: self.renders,
            generation: self.generation,
            current: Some(loss.total),
            best: self.candidates.first().cloned(),
            elapsed_seconds: self.start.elapsed().as_secs_f64(),
        });
    }
    fn active(&self, stage: FitStage) -> Vec<Parameter> {
        let mut params = searchable(
            &self.input.config,
            stage,
            &self.input.initial,
            &self.input.locks,
        );
        if self.context.audition.key_on_seconds >= self.context.audition.total_seconds {
            params.retain(|p| {
                !matches!(
                    p,
                    Parameter::Operator {
                        field: OperatorField::Rr,
                        ..
                    }
                )
            });
        }
        params
    }
    fn seeded(&self, seed: &Instrument) -> Instrument {
        let mut patch = self.input.initial.clone();
        if self
            .active(FitStage::Refine)
            .contains(&Parameter::Channel3Mode)
        {
            patch.channel3_special = seed.channel3_special;
        }
        for p in self
            .active(FitStage::Refine)
            .into_iter()
            .filter(|p| *p != Parameter::Channel3Mode)
        {
            if seed.channel3_special.is_none()
                && matches!(
                    p,
                    Parameter::SpecialFnum { .. } | Parameter::SpecialBlock { .. }
                )
            {
                continue;
            }
            p.set(&mut patch, p.get(seed));
        }
        patch
    }
    fn project(base: &Instrument, params: &[Parameter], genes: &[f64]) -> Instrument {
        let mut patch = base.clone();
        for (p, gene) in params.iter().zip(genes) {
            p.set(
                &mut patch,
                gene.round().clamp(0.0, f64::from(p.maximum())) as u16,
            );
        }
        patch
    }
    fn evolution(
        &mut self,
        params: &[Parameter],
        end: usize,
        random: &mut Random,
        cancel: &impl Fn() -> bool,
    ) -> Result<(), FitError> {
        if params.is_empty() || self.evaluations >= end {
            return Ok(());
        }
        let base = self.candidates[0].instrument.clone();
        let mut population = Vec::new();
        let size = self.input.config.population.min(end - self.evaluations);
        let mut families: Vec<_> = self.families.iter().flatten().collect();
        families.sort_by(|a, b| a.loss.total.total_cmp(&b.loss.total));
        let mut seeds = Vec::new();
        for seed in self
            .input
            .seeds
            .iter()
            .map(|s| self.seeded(s))
            .chain(families.iter().map(|c| c.instrument.clone()))
            .chain(self.candidates.iter().skip(1).map(|c| c.instrument.clone()))
        {
            if seed != base && !seeds.contains(&seed) {
                seeds.push(seed);
            }
        }
        let mut proposals = Vec::new();
        for i in 0..size {
            // At least half the population receives new variation.
            // Complete bases retain distinct algorithm families through later
            // envelope/modulation stages; external refinement seeds come first.
            let seed = if i > 0 && i < size.div_ceil(2) {
                seeds.get(i - 1)
            } else {
                None
            };
            let member_base = seed
                .unwrap_or_else(|| {
                    if i == 0 {
                        &base
                    } else {
                        seeds
                            .get(i.saturating_sub(1) % seeds.len().max(1))
                            .unwrap_or(&base)
                    }
                })
                .clone();
            let genes = params
                .iter()
                .map(|p| {
                    if i == 0 || seed.is_some() {
                        return f64::from(p.get(&member_base));
                    }
                    let maximum = f64::from(p.maximum());
                    if self.input.refine || (self.stage != FitStage::Coarse && i % 2 == 0) {
                        (f64::from(p.get(&member_base))
                            + (random.unit() * 2.0 - 1.0) * maximum * 0.2)
                            .clamp(0.0, maximum)
                    } else if let Parameter::Operator {
                        index,
                        field: OperatorField::Tl,
                    } = p
                    {
                        // Carrier TL controls output level; modulator TL controls timbre.
                        let carrier = member_base
                            .channel
                            .algorithm
                            .is_carrier(discofm_chip::OperatorId::ALL[usize::from(*index)]);
                        if carrier {
                            random.unit() * 80.0
                        } else {
                            16.0 + random.unit() * 96.0
                        }
                    } else {
                        random.unit() * maximum
                    }
                })
                .collect::<Vec<_>>();
            proposals.push((member_base, genes));
        }
        let patches = proposals
            .iter()
            .map(|(base, genes)| Self::project(base, params, genes))
            .collect();
        let losses = self.score_many(patches, cancel)?;
        let (f, cr) = (
            self.input.config.differential_weight,
            self.input.config.crossover,
        );
        for ((base, genes), loss) in proposals.into_iter().zip(losses) {
            population.push(Member {
                base,
                genes,
                loss,
                f,
                cr,
            });
        }
        let adaptive = self.input.config.adaptive;
        if population.len() < 4 {
            return Ok(());
        }
        while self.evaluations < end {
            check(cancel)?;
            self.generation += 1;
            let frozen = population.clone();
            let mut trials = Vec::new();
            let mut patches = Vec::new();
            let per_target = if self.screen.is_some() {
                self.input.config.screening.max(1)
            } else {
                1
            };
            for target in (0..population.len().min(end - self.evaluations))
                .flat_map(|target| std::iter::repeat_n(target, per_target))
            {
                let (mut f, mut cr) = (frozen[target].f, frozen[target].cr);
                // jDE (Brest et al. 2006): resample with probability 0.1, F in
                // 0.1..1; a member keeps the pair only if its trial wins. The
                // fixed mode draws nothing extra, so archived runs reproduce.
                if adaptive {
                    if random.unit() < 0.1 {
                        f = 0.1 + 0.9 * random.unit();
                    }
                    if random.unit() < 0.1 {
                        cr = random.unit();
                    }
                }
                let [a, b, c] = random.donors(population.len(), target);
                let forced = random.index(params.len());
                let mut trial = frozen[target].genes.clone();
                for j in 0..params.len() {
                    if j == forced || random.unit() < cr {
                        trial[j] = (frozen[a].genes[j]
                            + f * (frozen[b].genes[j] - frozen[c].genes[j]))
                            .clamp(0.0, f64::from(params[j].maximum()));
                    }
                }
                let member_base = &frozen[target].base;
                let mut patch = Self::project(member_base, params, &trial);
                if patch == Self::project(member_base, params, &frozen[target].genes) {
                    let direction = if random.unit() < 0.5 { -1.0 } else { 1.0 };
                    trial[forced] = (f64::from(params[forced].get(&patch)) + direction)
                        .clamp(0.0, f64::from(params[forced].maximum()));
                    patch = Self::project(member_base, params, &trial);
                }
                patches.push(patch);
                trials.push((target, member_base.clone(), trial, f, cr));
            }
            if per_target > 1
                && let Some(screen) = self.screen.as_mut()
            {
                check(cancel)?;
                // Keep each target's best-screened trial; ties keep the first.
                let scores = screen(&patches)?;
                let mut kept_trials = Vec::new();
                let mut kept_patches = Vec::new();
                for ((trials, patches), scores) in trials
                    .chunks(per_target)
                    .zip(patches.chunks(per_target))
                    .zip(scores.chunks(per_target))
                {
                    let best = (0..scores.len())
                        .min_by(|a, b| scores[*a].total_cmp(&scores[*b]).then(a.cmp(b)))
                        .unwrap_or(0);
                    kept_trials.push(trials[best].clone());
                    kept_patches.push(patches[best].clone());
                }
                trials = kept_trials;
                patches = kept_patches;
            }
            let losses = self.score_many(patches, cancel)?;
            for ((target, base, genes, f, cr), loss) in trials.into_iter().zip(losses) {
                if loss.total <= frozen[target].loss.total {
                    population[target] = Member {
                        base,
                        genes,
                        loss,
                        f,
                        cr,
                    };
                }
            }
        }
        Ok(())
    }
    fn coordinates(&mut self, end: usize, cancel: &impl Fn() -> bool) -> Result<(), FitError> {
        let mut params: Vec<_> = self
            .active(FitStage::Refine)
            .into_iter()
            .filter(|p| *p != Parameter::Channel3Mode)
            .collect();
        if params.is_empty() {
            return Ok(());
        }
        let algorithm = self.candidates[0].instrument.channel.algorithm;
        params.sort_by_key(|p| match p {
            Parameter::Operator {
                index,
                field: OperatorField::Tl,
            } if algorithm.is_carrier(discofm_chip::OperatorId::ALL[usize::from(*index)]) => 0,
            Parameter::Operator {
                field: OperatorField::D1r | OperatorField::D2r | OperatorField::Sl,
                ..
            } => 1,
            Parameter::Operator {
                field: OperatorField::Ar | OperatorField::Rr | OperatorField::Rs,
                ..
            } => 2,
            Parameter::Operator {
                field: OperatorField::Mul | OperatorField::Tl,
                ..
            } => 3,
            _ => 4,
        });
        // A sweep whose every move ymfm predicted worse renders nothing, so
        // the next sweep runs unscreened rather than repeating it forever.
        let mut unscreened = false;
        while self.evaluations < end {
            self.generation += 1;
            let screening = self.screen.is_some() && !unscreened;
            let (before, leader) = (self.evaluations, self.candidates[0].loss.total);
            let scales = if self.input.config.mode == FitMode::Advanced {
                [0.0625, 0.0, 0.25]
            } else {
                [0.0, 0.03125, 0.125]
            };
            let moves: Vec<_> = scales
                .into_iter()
                .flat_map(|scale| {
                    params.iter().map(move |p| {
                        let step = match p {
                            Parameter::Operator {
                                field:
                                    OperatorField::Ar
                                    | OperatorField::D1r
                                    | OperatorField::D2r
                                    | OperatorField::Rr
                                    | OperatorField::Sl
                                    | OperatorField::Rs,
                                ..
                            } if scale <= 0.0625 => 1,
                            Parameter::SpecialFnum { .. } if scale <= 0.0625 => 2,
                            _ => (f64::from(p.maximum()) * scale).round().max(1.0) as i32,
                        };
                        (*p, step)
                    })
                })
                .collect();
            let shift = |base: &Instrument, p: Parameter, step: i32, direction: i32| {
                let mut patch = base.clone();
                let value =
                    (i32::from(p.get(&patch)) + step * direction).clamp(0, i32::from(p.maximum()));
                p.set(&mut patch, value as u16);
                patch
            };
            // ymfm screens the leader and both directions of the next moves
            // from it, in short windows so the refresh after an improvement
            // stays cheap. Scores are cached: at a local optimum sweeps repeat
            // identical moves, and re-screening them cost more wall time than
            // the renders screening saved.
            let window = 2 * self.workers();
            let mut screened_from: Option<(Instrument, usize)> = None;
            for (index, &(p, step)) in moves.iter().enumerate() {
                let base = self.candidates[0].instrument.clone();
                if screening && let Some(screen) = self.screen.as_mut() {
                    if screened_from
                        .as_ref()
                        .is_none_or(|(b, from)| *b != base || index >= from + window)
                    {
                        check(cancel)?;
                        let mut wanted = std::collections::BTreeSet::new();
                        let batch: Vec<_> = std::iter::once(base.clone())
                            .chain(
                                moves[index..(index + window).min(moves.len())]
                                    .iter()
                                    .flat_map(|&(p, step)| {
                                        [-1, 1].map(|d| shift(&base, p, step, d))
                                    }),
                            )
                            .filter(|patch| {
                                let identity = key(patch);
                                !self.screen_cache.contains_key(&identity)
                                    && wanted.insert(identity)
                            })
                            .collect();
                        if !batch.is_empty() {
                            let scores = screen(&batch)?;
                            for (patch, score) in batch.iter().zip(scores) {
                                self.screen_cache.insert(key(patch), score);
                            }
                        }
                        screened_from = Some((base.clone(), index));
                    }
                    let score = |patch: &Instrument| {
                        self.screen_cache
                            .get(&key(patch))
                            .copied()
                            .unwrap_or(f64::NEG_INFINITY)
                    };
                    let leader = score(&base);
                    if score(&shift(&base, p, step, -1)) >= leader
                        && score(&shift(&base, p, step, 1)) >= leader
                    {
                        continue;
                    }
                }
                let mut patches = Vec::new();
                for direction in [-1, 1] {
                    if self.evaluations + patches.len() >= end {
                        break;
                    }
                    patches.push(shift(&base, p, step, direction));
                }
                // The next moves from the same leader, valid until one improves.
                let room = end.saturating_sub(self.evaluations + patches.len());
                let lookahead = moves[index + 1..]
                    .iter()
                    .flat_map(|&(p, step)| [-1, 1].map(|d| shift(&base, p, step, d)))
                    .take(room.min(2 * self.workers()))
                    .collect();
                self.score_ahead(patches, lookahead, cancel)?;
            }
            if screening && self.evaluations == before {
                unscreened = true;
            } else if self.candidates[0].loss.total < leader {
                unscreened = false;
            }
        }
        Ok(())
    }
    fn carrier_envelopes(
        &mut self,
        end: usize,
        refining: bool,
        cancel: &impl Fn() -> bool,
    ) -> Result<(), FitError> {
        let active = self.active(FitStage::Refine);
        // Coherent carrier moves fit the overall decay before spending the
        // remaining budget on independent operator evolution. In additive
        // algorithms, moving one of four equal envelopes barely changes RMS.
        let mut fields = if refining {
            vec![
                (OperatorField::D1r, &[-2, -1, 1, 2][..]),
                (OperatorField::Sl, &[-1, 1][..]),
                (OperatorField::D2r, &[-1, 1][..]),
                (OperatorField::Ar, &[-2, -1, 1, 2][..]),
            ]
        } else {
            vec![
                (OperatorField::D1r, &[-8, -4, -2, -1, 1, 2, 4, 8][..]),
                (OperatorField::Sl, &[-2, -1, 1, 2][..]),
                (OperatorField::D2r, &[-4, -1, 1, 4][..]),
                (OperatorField::Ar, &[-4, -2, -1, 1, 2, 4][..]),
            ]
        };
        if active.iter().any(|p| {
            matches!(
                p,
                Parameter::Operator {
                    field: OperatorField::Rr,
                    ..
                }
            )
        }) {
            fields.push((
                OperatorField::Rr,
                if refining {
                    &[-2, -1, 1, 2][..]
                } else {
                    &[-4, -2, -1, 1, 2, 4][..]
                },
            ));
        }
        let best = &self.candidates[0].instrument;
        let family_index = |p: &Instrument| {
            usize::from(p.channel.algorithm.get()) + usize::from(p.channel3_special.is_some()) * 8
        };
        let mut bases = vec![family_index(best)];
        if !refining
            && let Some(other) = self
                .families
                .iter()
                .flatten()
                .filter(|c| {
                    c.instrument.channel3_special.is_some() != best.channel3_special.is_some()
                })
                .min_by(|a, b| a.loss.total.total_cmp(&b.loss.total))
        {
            bases.push(family_index(&other.instrument));
        }
        // Fit a promising opposite topology before comparing it with the leader;
        // otherwise its initial envelope can eliminate an accurate partial layout.
        for _ in 0..if refining || bases.len() > 1 { 1 } else { 2 } {
            for (position, family) in bases.iter().enumerate() {
                // Reserve a fair share for the opposite topology rather than
                // spending the whole envelope budget on the current leader.
                let family_end = self.evaluations
                    + end.saturating_sub(self.evaluations) / (bases.len() - position);
                let Some(origin) = self.family(*family) else {
                    continue;
                };
                let base = origin.clone();
                let audible: Vec<_> = (0..4)
                    .filter(|&index| {
                        base.channel
                            .algorithm
                            .is_carrier(discofm_chip::OperatorId::ALL[index])
                            && base.operators[index].tl.get() < 127
                    })
                    .collect();
                let minimum_tl = audible
                    .iter()
                    .map(|&i| i32::from(base.operators[i].tl.get()))
                    .min()
                    .unwrap_or(0);
                let mut levels = Vec::new();
                for offset in if refining {
                    if minimum_tl > 8 {
                        vec![-minimum_tl, -24, -16, -8]
                    } else {
                        Vec::new()
                    }
                } else {
                    vec![-minimum_tl, -24, -16, -8, 8, 16]
                } {
                    let mut patch = base.clone();
                    for &index in &audible {
                        let p = Parameter::Operator {
                            index: index as u8,
                            field: OperatorField::Tl,
                        };
                        if active.contains(&p) {
                            p.set(
                                &mut patch,
                                (i32::from(p.get(&base)) + offset).clamp(0, 127) as u16,
                            );
                        }
                    }
                    if patch != base && !levels.contains(&patch) {
                        levels.push(patch);
                    }
                }
                // A shared recording gain cannot remove the chip's DAC floor.
                // Coherent TL moves preserve partial balance while finding useful
                // chip headroom instead of amplifying quantization with the signal.
                levels.truncate(family_end.saturating_sub(self.evaluations));
                // Removing a redundant carrier is a discrete topology choice:
                // small TL steps cannot reach silence within a modest budget.
                // In special mode a spare note-frequency carrier can otherwise
                // beat against an independently tuned partial near the root.
                let masks = |base: &Instrument| {
                    let mut masks = Vec::new();
                    if !refining && audible.len() > 1 {
                        for &index in &audible {
                            let p = Parameter::Operator {
                                index: index as u8,
                                field: OperatorField::Tl,
                            };
                            if active.contains(&p) {
                                let mut patch = base.clone();
                                p.set(&mut patch, 127);
                                masks.push(patch);
                            }
                        }
                    }
                    masks
                };
                let shared = |base: &Instrument, field: OperatorField, steps: &[i32]| {
                    let mut patches = Vec::new();
                    for step in steps {
                        let mut patch = base.clone();
                        for index in 0..4 {
                            let p = Parameter::Operator { index, field };
                            if active.contains(&p)
                                && base.operators[index as usize].tl.get() < 127
                                && base
                                    .channel
                                    .algorithm
                                    .is_carrier(discofm_chip::OperatorId::ALL[index as usize])
                            {
                                let value = (i32::from(p.get(base)) + step)
                                    .clamp(0, i32::from(p.maximum()))
                                    as u16;
                                p.set(&mut patch, value);
                            }
                        }
                        if patch != *base && !patches.contains(&patch) {
                            patches.push(patch);
                        }
                    }
                    patches
                };
                // Later moves from an unchanged family leader, scored by idle workers.
                let ahead = |base: &Instrument, from: usize, with_masks: bool, used: usize| {
                    let mut patches = if with_masks { masks(base) } else { Vec::new() };
                    for &(field, steps) in &fields[from..] {
                        patches.extend(shared(base, field, steps));
                    }
                    patches.truncate(end.saturating_sub(used));
                    patches
                };
                let lookahead = ahead(&base, 0, true, self.evaluations);
                self.score_ahead(levels, lookahead, cancel)?;
                let base = self.family(*family).unwrap_or_else(|| origin.clone());
                let mut patches = masks(&base);
                patches.truncate(family_end.saturating_sub(self.evaluations));
                let lookahead = ahead(&base, 0, false, self.evaluations);
                self.score_ahead(patches, lookahead, cancel)?;
                for (index, &(field, steps)) in fields.iter().enumerate() {
                    let base = self.family(*family).unwrap_or_else(|| origin.clone());
                    let mut patches = shared(&base, field, steps);
                    patches.truncate(family_end.saturating_sub(self.evaluations));
                    let lookahead = ahead(&base, index + 1, false, self.evaluations);
                    self.score_ahead(patches, lookahead, cancel)?;
                }
            }
        }
        Ok(())
    }
    fn modulation_depths(
        &mut self,
        end: usize,
        cancel: &impl Fn() -> bool,
    ) -> Result<(), FitError> {
        let mut base = self.candidates[0].instrument.clone();
        if self.input.config.mode != FitMode::Advanced || !base.global.lfo_enabled {
            return Ok(());
        }
        let active = self.active(FitStage::Refine);
        if !base.operators.iter().any(|op| op.am) && active.contains(&Parameter::Ams) {
            for index in 0..4 {
                let p = Parameter::Operator {
                    index,
                    field: OperatorField::Am,
                };
                if active.contains(&p)
                    && base
                        .channel
                        .algorithm
                        .is_carrier(discofm_chip::OperatorId::ALL[index as usize])
                {
                    p.set(&mut base, 1);
                }
            }
        }
        let am = active.contains(&Parameter::Ams) && base.operators.iter().any(|op| op.am);
        let fm = active.contains(&Parameter::Fms);
        if !am && !fm {
            return Ok(());
        }
        // AMS and FMS interact with the single recording gain. Joint enumeration
        // avoids a coordinate minimum at an excessive AM depth and covers every
        // hardware sensitivity instead of relying on the structured starting depth.
        let mut patches = Vec::new();
        for ams in 0..if am { 4 } else { 1 } {
            for fms in 0..if fm { 8 } else { 1 } {
                let mut patch = base.clone();
                if am {
                    Parameter::Ams.set(&mut patch, ams);
                }
                if fm {
                    Parameter::Fms.set(&mut patch, fms);
                }
                if patch != base {
                    patches.push(patch);
                }
            }
        }
        patches.truncate(end.saturating_sub(self.evaluations));
        self.score_many(patches, cancel)?;
        Ok(())
    }
    /// Hardware idioms that DE rarely assembles from random genes: additive
    /// harmonic series on parallel carriers (square/saw), self-feedback buzz and
    /// the feedback-7, high-MUL chaotic modulator behind FM snares and hats.
    /// Each shape gets a sustained and a percussive envelope; locks still hold.
    fn timbre_seeds(&self) -> Vec<Instrument> {
        let carrier_tl = if self.input.config.match_level { 0 } else { 28 };
        // (algorithm, feedback, S1..S4 (MUL, TL)); TL 0.75 dB/step, so 1/n ≈ 26.6·log10(n).
        let shapes: [(u16, u16, [Ratio; 4]); 7] = [
            (7, 0, [(1, 0), (3, 13), (5, 19), (7, 23)]),
            (7, 0, [(1, 0), (2, 8), (3, 13), (4, 16)]),
            // Quiet, not muted, companions: fully silent operators make every
            // later mutation inaudible, a plateau DE and coordinates cannot leave.
            // Carrier removal in the envelope stage can still mute them.
            (7, 6, [(1, 0), (2, 40), (3, 48), (4, 56)]),
            (7, 7, [(1, 0), (2, 40), (3, 48), (4, 56)]),
            (4, 7, [(15, 0), (1, 0), (1, 24), (1, 0)]),
            (5, 7, [(15, 8), (1, 0), (1, 0), (1, 0)]),
            (6, 7, [(12, 0), (1, 0), (1, 0), (1, 0)]),
        ];
        let mut result = Vec::new();
        for (algorithm, feedback, operators) in shapes {
            for (d1r, sl) in [(0, 0), (16, 15)] {
                let mut patch = self.input.initial.clone();
                Parameter::Algorithm.set(&mut patch, algorithm);
                Parameter::Feedback.set(&mut patch, feedback);
                for (index, (mul, tl)) in (0..4).zip(operators) {
                    let carrier = patch
                        .channel
                        .algorithm
                        .is_carrier(discofm_chip::OperatorId::ALL[usize::from(index)]);
                    let tl = if carrier && tl < 127 {
                        tl + carrier_tl
                    } else {
                        tl
                    };
                    for (field, value) in [
                        (OperatorField::Mul, mul),
                        (OperatorField::Tl, tl),
                        (OperatorField::Dt, 0),
                        (OperatorField::Ar, 31),
                        (OperatorField::D1r, d1r),
                        (OperatorField::Sl, sl),
                        (OperatorField::D2r, 0),
                        (OperatorField::Rs, 0),
                        (OperatorField::Ssg, 0),
                    ] {
                        Parameter::Operator { index, field }.set(&mut patch, value);
                    }
                }
                let patch = self.seeded(&patch);
                if !result.contains(&patch) {
                    result.push(patch);
                }
            }
        }
        result
    }
    /// MUL and FB change timbre discontinuously; ±1/±4 coordinate steps cannot
    /// reach distant ratios such as a 1→15 noise modulator or 1→7 odd partials.
    fn ratio_sweep(&mut self, end: usize, cancel: &impl Fn() -> bool) -> Result<(), FitError> {
        let active = self.active(FitStage::Refine);
        let base = self.candidates[0].instrument.clone();
        let mut patches = Vec::new();
        for index in 0..4 {
            let p = Parameter::Operator {
                index,
                field: OperatorField::Mul,
            };
            // A silent operator's ratio is inaudible; skip it rather than spend budget.
            if !active.contains(&p) || base.operators[usize::from(index)].tl.get() == 127 {
                continue;
            }
            let current = i32::from(p.get(&base));
            for value in [0, 1, 2, 3, 5, 7, 10, 15] {
                if (i32::from(value) - current).abs() > 1 {
                    let mut patch = base.clone();
                    p.set(&mut patch, value);
                    patches.push(patch);
                }
            }
        }
        if active.contains(&Parameter::Feedback) {
            let current = i32::from(Parameter::Feedback.get(&base));
            for value in [0, 3, 5, 6, 7] {
                if (i32::from(value) - current).abs() > 1 {
                    let mut patch = base.clone();
                    Parameter::Feedback.set(&mut patch, value);
                    patches.push(patch);
                }
            }
        }
        patches.truncate(end.saturating_sub(self.evaluations));
        self.score_many(patches, cancel)?;
        Ok(())
    }
    fn seed_partials(&self, patch: &mut Instrument, active: &[Parameter]) {
        if self.anchors.is_empty() {
            return;
        }
        let multiplier = |index: usize| {
            let value = patch.operators[index].mul.get();
            if value == 0 { 0.5 } else { f64::from(value) }
        };
        let root_hz = self.context.pitch_hz * multiplier(3);
        let root_audible = patch.operators[3].tl.get() < 127
            || active.contains(&Parameter::Operator {
                index: 3,
                field: OperatorField::Tl,
            });
        let root = self
            .anchors
            .iter()
            .enumerate()
            .filter(|(_, p)| root_audible && (1200.0 * (p.hz / root_hz).log2()).abs() < 180.0)
            .min_by(|(_, a), (_, b)| {
                (a.hz / root_hz)
                    .ln()
                    .abs()
                    .total_cmp(&(b.hz / root_hz).ln().abs())
            })
            .map(|(i, _)| i);
        let mut remaining: Vec<_> = (0..self.anchors.len())
            .filter(|i| Some(*i) != root)
            .collect();
        let mut assignments = [None; 4];
        assignments[3] = root;
        for (index, assigned) in assignments.iter_mut().enumerate().take(3) {
            if patch
                .channel
                .algorithm
                .is_carrier(discofm_chip::OperatorId::ALL[index])
                && (patch.operators[index].tl.get() < 127
                    || active.contains(&Parameter::Operator {
                        index: index as u8,
                        field: OperatorField::Tl,
                    }))
                && !remaining.is_empty()
            {
                let anchor = remaining.remove(0);
                if let Ok(frequency) = discofm_engine::frequency_for_hz(
                    self.anchors[anchor].hz / multiplier(index),
                    self.context.audition.clock,
                ) && let Some(frequencies) = patch.channel3_special.as_mut()
                {
                    frequencies[index] = frequency;
                    *assigned = Some(anchor);
                }
            }
        }
        if assignments[..3].iter().all(Option::is_none) {
            return;
        }
        let maximum = self.anchors[0].amplitude.max(1e-20);
        for (index, assigned) in assignments.into_iter().enumerate() {
            let p = Parameter::Operator {
                index: index as u8,
                field: OperatorField::Tl,
            };
            if !active.contains(&p)
                || !patch
                    .channel
                    .algorithm
                    .is_carrier(discofm_chip::OperatorId::ALL[index])
            {
                continue;
            }
            let value = assigned.map_or(127, |anchor| {
                let relative = (-20.0
                    * (self.anchors[anchor].amplitude / maximum)
                        .max(1e-20)
                        .log10()
                    / 0.75)
                    .round();
                let base = if self.input.config.match_level {
                    0.0
                } else {
                    f64::from(p.get(patch))
                };
                (base + relative).clamp(0.0, 127.0) as u16
            });
            p.set(patch, value);
        }
    }
    fn mode_proposals(
        &mut self,
        end: usize,
        modulation: bool,
        cancel: &impl Fn() -> bool,
    ) -> Result<(), FitError> {
        let active = self.active(FitStage::Refine);
        let mut proposals = Vec::new();
        if !modulation && active.contains(&Parameter::Channel3Mode) {
            let bases: Vec<_> = self
                .families
                .iter()
                .flatten()
                .take(8)
                .map(|c| c.instrument.clone())
                .collect();
            for variant in 0..2 {
                for base in &bases {
                    let mut patch = base.clone();
                    if variant == 0 && base.channel3_special.is_some() {
                        patch.channel3_special = None;
                    } else {
                        let root = discofm_engine::frequency_for_hz(
                            self.context.pitch_hz,
                            self.context.audition.clock,
                        )?;
                        let frequencies = [root; 3];
                        patch.channel3_special = Some(frequencies);
                        if variant == 1 {
                            self.seed_partials(&mut patch, &active);
                        }
                    }
                    proposals.push(self.seeded(&patch));
                }
            }
        } else if modulation {
            let base = self.candidates[0].instrument.clone();
            // Gates are proposed together: an inactive single-bit mutation is
            // often acoustically identical and cannot cross the fitness plateau.
            for kind in 0..4 {
                for rate in 0..if kind == 0 { 1 } else { 8 } {
                    let mut patch = base.clone();
                    for (p, value) in [
                        (Parameter::LfoEnabled, u16::from(kind != 0)),
                        (Parameter::LfoRate, rate),
                        (Parameter::Ams, if kind >= 2 { 2 } else { 0 }),
                        (Parameter::Fms, if kind == 1 || kind == 3 { 5 } else { 0 }),
                    ] {
                        if active.contains(&p) {
                            p.set(&mut patch, value);
                        }
                    }
                    for index in 0..4 {
                        let p = Parameter::Operator {
                            index,
                            field: OperatorField::Am,
                        };
                        if active.contains(&p) {
                            p.set(
                                &mut patch,
                                u16::from(
                                    kind >= 2
                                        && base.channel.algorithm.is_carrier(
                                            discofm_chip::OperatorId::ALL[index as usize],
                                        ),
                                ),
                            );
                        }
                    }
                    if patch != base && !proposals.contains(&patch) {
                        proposals.push(patch);
                    }
                }
            }
            // All SSG categories, including hold and alternating envelopes.
            let carrier = (0..4)
                .find(|&i| {
                    base.channel
                        .algorithm
                        .is_carrier(discofm_chip::OperatorId::ALL[i])
                })
                .unwrap_or(3);
            for value in 0..=8 {
                let mut patch = base.clone();
                let p = Parameter::Operator {
                    index: carrier as u8,
                    field: OperatorField::Ssg,
                };
                if active.contains(&p) {
                    p.set(&mut patch, value);
                    if patch != base {
                        proposals.push(patch);
                    }
                }
            }
            // AM on modulators explores periodic timbre, independently of tremolo.
            for rate in [1, 4, 7] {
                let mut patch = base.clone();
                for (p, value) in [
                    (Parameter::LfoEnabled, 1),
                    (Parameter::LfoRate, rate),
                    (Parameter::Ams, 2),
                ] {
                    if active.contains(&p) {
                        p.set(&mut patch, value);
                    }
                }
                for index in 0..4 {
                    let p = Parameter::Operator {
                        index,
                        field: OperatorField::Am,
                    };
                    if active.contains(&p) {
                        p.set(
                            &mut patch,
                            u16::from(
                                !base
                                    .channel
                                    .algorithm
                                    .is_carrier(discofm_chip::OperatorId::ALL[index as usize]),
                            ),
                        );
                    }
                }
                if patch != base && !proposals.contains(&patch) {
                    proposals.push(patch);
                }
            }
        }
        proposals.truncate(end.saturating_sub(self.evaluations));
        if !proposals.is_empty() {
            self.score_many(proposals, cancel)?;
        }
        Ok(())
    }
    fn enumerates_algorithms(&self) -> bool {
        !self.input.refine && !self.input.locks.contains(&Parameter::Algorithm)
    }
    fn algorithm_seeds(&self) -> Vec<Instrument> {
        let mut seeds = Vec::new();
        for algorithm in 0..8 {
            let mut patch = self.input.initial.clone();
            Parameter::Algorithm.set(&mut patch, algorithm);
            for index in 0..4 {
                let p = Parameter::Operator {
                    index,
                    field: OperatorField::Tl,
                };
                if !self.input.locks.contains(&p) {
                    let carrier = patch
                        .channel
                        .algorithm
                        .is_carrier(discofm_chip::OperatorId::ALL[usize::from(index)]);
                    p.set(
                        &mut patch,
                        if carrier {
                            self.input.initial.operators[3].tl.get().into()
                        } else {
                            64
                        },
                    );
                }
            }
            seeds.push(patch);
        }
        seeds
    }
    fn run(&mut self, cancel: &impl Fn() -> bool) -> Result<(), FitError> {
        let searchable = !self.active(FitStage::Refine).is_empty();
        let lookahead = if searchable && self.enumerates_algorithms() {
            self.algorithm_seeds()
        } else {
            Vec::new()
        };
        self.score_ahead(vec![self.input.initial.clone()], lookahead, cancel)?;
        if !searchable {
            return Ok(());
        }
        let mut random = Random(self.input.config.seed);
        let budget = self.input.config.evaluations;
        let stages = match self.input.config.mode {
            FitMode::Basic => vec![
                (FitStage::Coarse, (budget * 7 / 10).max(10)),
                (FitStage::Refine, budget),
            ],
            FitMode::Advanced => vec![
                (FitStage::Coarse, (budget * 30 / 100).max(10)),
                (FitStage::Envelopes, budget * 55 / 100),
                (FitStage::Modulation, budget * 80 / 100),
                (FitStage::Refine, budget),
            ],
        };
        for (stage, end) in stages {
            self.stage = stage;
            check(cancel)?;
            if stage == FitStage::Coarse && self.enumerates_algorithms() {
                let mut seeds = self.algorithm_seeds();
                // Structured ratio/envelope starts avoid choosing an algorithm
                // solely under the initial patch's (often flat) envelopes.
                // Rebase through the searchable set so locks and Basic mode hold.
                let variants = ((end / 3).saturating_sub(9) / 8).min(6);
                for variant in 0..variants {
                    for algorithm in 0..8 {
                        let mut patch = self.input.initial.clone();
                        Parameter::Algorithm.set(&mut patch, algorithm);
                        Parameter::Feedback.set(&mut patch, if variant % 3 == 2 { 5 } else { 0 });
                        for index in 0..4 {
                            let carrier = patch
                                .channel
                                .algorithm
                                .is_carrier(discofm_chip::OperatorId::ALL[index as usize]);
                            for (field, value) in [
                                (
                                    OperatorField::Mul,
                                    if carrier {
                                        1
                                    } else {
                                        [1, 2, 3, 1, 2, 4][variant]
                                    },
                                ),
                                (
                                    OperatorField::Tl,
                                    if carrier {
                                        if self.input.config.match_level { 0 } else { 28 }
                                    } else {
                                        [40, 48, 56, 32, 40, 48][variant]
                                    },
                                ),
                                (OperatorField::Dt, 0),
                                (OperatorField::Ar, 31),
                                (OperatorField::D1r, if variant < 3 { 14 } else { 22 }),
                                (OperatorField::Sl, 8),
                                (OperatorField::D2r, 4),
                                (OperatorField::Rr, 10),
                                (OperatorField::Rs, 0),
                            ] {
                                Parameter::Operator { index, field }.set(&mut patch, value);
                            }
                        }
                        let patch = self.seeded(&patch);
                        seeds.push(patch);
                    }
                }
                // Idioms that win early under crude envelopes can steer DE away
                // from better layouts; keep four DE generations first. At 256
                // queries this admits none (SteelDrum regressed), at 512 all.
                let room = end.saturating_sub(
                    self.evaluations + seeds.len() + 4 * self.input.config.population,
                );
                for patch in self.timbre_seeds().into_iter().take(room) {
                    if !seeds.contains(&patch) {
                        seeds.push(patch);
                    }
                }
                self.score_many(seeds, cancel)?;
            }
            if stage == FitStage::Coarse {
                self.mode_proposals(
                    end.saturating_sub(self.input.config.population),
                    false,
                    cancel,
                )?;
            }
            if stage == FitStage::Modulation {
                self.mode_proposals(
                    end.saturating_sub(self.input.config.population),
                    true,
                    cancel,
                )?;
            }
            if stage == FitStage::Envelopes {
                self.carrier_envelopes(
                    end.saturating_sub(self.input.config.population),
                    false,
                    cancel,
                )?;
            }
            if stage == FitStage::Refine {
                self.ratio_sweep(end.saturating_sub(48).max(self.evaluations), cancel)?;
                if self.input.config.mode == FitMode::Advanced {
                    // Revisit coupled levels/envelopes after individual moves;
                    // late mutations can otherwise fall into a DAC-floor minimum.
                    self.coordinates(end.saturating_sub(24).max(self.evaluations), cancel)?;
                    self.carrier_envelopes(end, true, cancel)?;
                }
                self.coordinates(end, cancel)?;
            } else {
                let params: Vec<_> = self
                    .active(stage)
                    .into_iter()
                    .filter(|p| *p != Parameter::Channel3Mode)
                    .collect();
                let evolution_end = if stage == FitStage::Modulation
                    && self.candidates[0].instrument.global.lfo_enabled
                {
                    end.saturating_sub(32).max(self.evaluations)
                } else {
                    end
                };
                self.evolution(&params, evolution_end, &mut random, cancel)?;
                if stage == FitStage::Modulation {
                    self.modulation_depths(end, cancel)?;
                }
            }
        }
        Ok(())
    }
}
fn optimize<'a>(
    input: &'a FitInput,
    context: FitContext,
    cancel: &impl Fn() -> bool,
    evaluate: impl FnMut(&[Instrument]) -> Result<Vec<LossBreakdown>, FitError>,
    screen: Option<Screen<'a>>,
    anchors: Vec<crate::tracking::Peak>,
    progress: impl FnMut(FitProgress),
) -> Result<FitOutcome, FitError> {
    let mut search = Search {
        input,
        evaluate,
        screen,
        screen_cache: BTreeMap::new(),
        progress,
        cache: BTreeMap::new(),
        speculative: BTreeMap::new(),
        candidates: Vec::new(),
        families: std::array::from_fn(|_| None),
        history: History::new(),
        initial_loss: None,
        evaluations: 0,
        renders: 0,
        generation: 0,
        current: 0.0,
        stage: FitStage::Coarse,
        start: Instant::now(),
        context: context.clone(),
        anchors,
    };
    let status = match search.run(cancel) {
        Ok(()) => FitStatus::Completed,
        Err(FitError::Cancelled) if search.initial_loss.is_some() => FitStatus::Cancelled,
        Err(error) => return Err(error),
    };
    if search
        .history
        .points
        .last()
        .is_none_or(|p| p.evaluation != search.evaluations)
    {
        search.history.push(HistoryPoint {
            evaluation: search.evaluations,
            stage: search.stage,
            best: search.candidates[0].loss.total,
            current: search.current,
        });
    }
    let outcome = FitOutcome {
        status,
        config: input.config.clone(),
        locks: input.locks.clone(),
        initial: input.initial.clone(),
        context,
        initial_loss: search
            .initial_loss
            .ok_or_else(|| FitError::Internal("no baseline score was recorded".into()))?,
        candidates: search.candidates,
        history: search.history.points,
        evaluations: search.evaluations,
        renders: search.renders,
        generations: search.generation,
        elapsed_seconds: search.start.elapsed().as_secs_f64(),
        refined: input.refine,
        performance: crate::FitPerformance {
            cache_hits: search.evaluations - search.renders,
            cache_entries: search.cache.len(),
            ..Default::default()
        },
    };
    outcome.validate()?;
    Ok(outcome)
}
pub(crate) fn fit(
    input: FitInput,
    cancel: &impl Fn() -> bool,
    mut progress: impl FnMut(FitProgress),
) -> Result<FitOutcome, FitError> {
    let started = Instant::now();
    progress(FitProgress {
        stage: FitStage::Preparing,
        evaluations: 0,
        budget: input.config.evaluations,
        renders: 0,
        generation: 0,
        current: None,
        best: None,
        elapsed_seconds: 0.0,
    });
    let objective = std::cell::RefCell::new(Objective::prepare(&input, cancel)?);
    let context = objective.borrow().context.clone();
    let anchors = objective.borrow().partial_seeds.clone();
    let screen: Option<Screen> = (input.config.screening > 0).then(|| {
        Box::new(|patches: &[Instrument]| objective.borrow_mut().screen_batch(patches, cancel))
            as Screen
    });
    let mut outcome = optimize(
        &input,
        context,
        cancel,
        |patches| objective.borrow_mut().evaluate_batch(patches, cancel),
        screen,
        anchors,
        |mut p| {
            p.elapsed_seconds = started.elapsed().as_secs_f64();
            progress(p);
        },
    )?;
    outcome.elapsed_seconds = started.elapsed().as_secs_f64();
    let mut objective = objective.into_inner();
    objective.performance.cache_hits = outcome.performance.cache_hits;
    objective.performance.cache_entries = outcome.performance.cache_entries;
    outcome.performance = objective.performance;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_history_thins_evenly_and_keeps_stage_starts() {
        let total = 20_000;
        let stage = |e: usize| match e * 100 / total {
            0..30 => FitStage::Coarse,
            30..55 => FitStage::Envelopes,
            55..80 => FitStage::Modulation,
            _ => FitStage::Refine,
        };
        let mut history = History::new();
        for evaluation in 1..=total {
            let best = 10.0 - (evaluation / 50) as f64 * 0.01;
            history.record(HistoryPoint {
                evaluation,
                stage: stage(evaluation),
                best,
                current: best + 1.0,
            });
        }
        let points = &history.points;
        assert!(points.len() <= MAX_HISTORY);
        assert_eq!(points[0].evaluation, 1);
        for start in [6_000, 11_000, 16_000] {
            assert!(points.iter().any(|p| p.evaluation == start), "{start}");
        }
        let gap = points
            .windows(2)
            .map(|w| w[1].evaluation - w[0].evaluation)
            .max()
            .unwrap();
        // Dropping the oldest point instead left one gap of about 17000.
        assert!(gap <= 8 * total / MAX_HISTORY, "largest gap {gap}");
        assert!(points.windows(2).all(|w| w[1].best <= w[0].best));
    }
    #[test]
    fn splitmix_sequence_and_de_donors_are_stable_and_distinct() {
        let mut rng = Random(0);
        assert_eq!(rng.next(), 0xe220a8397b1dcdaf);
        assert_eq!(rng.next(), 0x6e789e6aa1b965f4);
        for size in 4..=64 {
            for target in 0..size {
                let donors = rng.donors(size, target);
                assert!(!donors.contains(&target));
                assert!(donors.iter().all(|d| *d < size));
                assert!(donors[0] != donors[1] && donors[1] != donors[2] && donors[0] != donors[2]);
            }
        }
    }
    fn fixture() -> (FitInput, FitContext) {
        let input = FitInput {
            reference: std::sync::Arc::new(discofm_audio::AudioBuffer {
                sample_rate: 24000,
                frames: vec![[0.1; 2]; 2400],
            }),
            options: Default::default(),
            note_on_seconds: 0.0,
            key_off_seconds: 0.05,
            initial: Instrument::default(),
            audition: discofm_engine::RenderRequest {
                total_seconds: 0.1,
                key_on_seconds: 0.05,
                ..Default::default()
            },
            config: crate::FitConfig {
                evaluations: 256,
                population: 8,
                ..Default::default()
            },
            locks: Default::default(),
            seeds: Vec::new(),
            refine: false,
        };
        let context = FitContext {
            loss_version: crate::LOSS_VERSION,
            fingerprint: "1234567890abcdef".into(),
            sample_rate: 24000,
            frames: 2400,
            reference_onset_seconds: 0.0,
            reference_start_seconds: 0.0,
            reference_end_seconds: 0.1,
            reference_gain: 1.0,
            pitch_hz: 440.0,
            audition: input.effective_audition(),
        };
        (input, context)
    }
    fn scalar_loss(value: f64) -> LossBreakdown {
        LossBreakdown {
            pitch: value,
            partials: value,
            periodicity: value,
            recording_gain: 1.0,
            total: value,
            log_spectral: value,
            convergence: value,
            envelope: value,
            harmonic: value,
            transient: value,
            modulation: value,
            tail: value,
            bands: value,
            inharmonic: value,
        }
    }
    #[test]
    fn coarse_and_refine_propose_square_buzz_noise_and_distant_ratios() {
        let (mut input, context) = fixture();
        input.config.evaluations = 512;
        let mut proposals = Vec::new();
        optimize(
            &input,
            context,
            &|| false,
            |patches| {
                proposals.extend(patches.iter().cloned());
                Ok(patches.iter().map(|_| scalar_loss(1.0)).collect())
            },
            None,
            Vec::new(),
            |_| {},
        )
        .unwrap();
        let has = |algorithm: u8, feedback: u8, muls: [u8; 4]| {
            proposals.iter().any(|p| {
                p.channel.algorithm.get() == algorithm
                    && p.channel.feedback.get() == feedback
                    && p.operators.each_ref().map(|o| o.mul.get()) == muls
            })
        };
        assert!(has(7, 0, [1, 3, 5, 7]), "odd additive square");
        assert!(has(7, 0, [1, 2, 3, 4]), "additive saw");
        assert!(has(7, 7, [1, 2, 3, 4]), "feedback buzz");
        assert!(has(4, 7, [15, 1, 1, 1]), "feedback noise modulator");
        // Equal scores keep the initial patch as leader for the refine sweep.
        let mut distant = input.initial.clone();
        Parameter::Operator {
            index: 3,
            field: OperatorField::Mul,
        }
        .set(&mut distant, 15);
        assert!(proposals.contains(&distant), "distant MUL ratio sweep");
    }
    #[test]
    fn all_stages_preserve_nondefault_locks_in_every_proposal_and_reject_forged_history() {
        let (mut input, context) = fixture();
        let params = crate::all_parameters(false);
        for (i, p) in params.iter().enumerate() {
            if *p != Parameter::Channel3Mode {
                p.set(&mut input.initial, (p.maximum() / 2).max(1));
            }
            if i % 2 == 0 {
                input.locks.insert(*p);
            }
        }
        let mut adversarial = Instrument::default();
        for p in &params {
            p.set(&mut adversarial, 0);
        }
        input.seeds.push(adversarial);
        let mut stages = Vec::new();
        let mut proposals = 0;
        let outcome = optimize(
            &input,
            context,
            &|| false,
            |patches| {
                patches
                    .iter()
                    .map(|patch| {
                        proposals += 1;
                        for lock in &input.locks {
                            assert_eq!(lock.get(patch), lock.get(&input.initial), "{lock:?}");
                        }
                        let value = params
                            .iter()
                            .filter(|p| !input.locks.contains(p))
                            .map(|p| {
                                (f64::from(p.get(patch)) / f64::from(p.maximum()) - 0.75).powi(2)
                            })
                            .sum::<f64>();
                        Ok(scalar_loss(value))
                    })
                    .collect()
            },
            None,
            Vec::new(),
            |p| {
                if !stages.contains(&p.stage) {
                    stages.push(p.stage);
                }
            },
        )
        .unwrap();
        assert_eq!(outcome.evaluations, 256);
        assert!(outcome.generations > 0 && proposals > 50);
        for stage in [
            FitStage::Coarse,
            FitStage::Envelopes,
            FitStage::Modulation,
            FitStage::Refine,
        ] {
            assert!(stages.contains(&stage));
        }
        assert!(outcome.candidates[0].loss.total < outcome.initial_loss.total);
        let mut forged = outcome;
        for p in &mut forged.history {
            p.best = 0.0;
        }
        assert!(forged.validate().is_err());
    }
    #[test]
    fn all_algorithms_are_evaluated_and_external_seeds_are_used_with_population_four() {
        let (mut input, context) = fixture();
        input.config.mode = FitMode::Basic;
        input.config.evaluations = 96;
        input.config.population = 4;
        let mul = Parameter::Operator {
            index: 3,
            field: OperatorField::Mul,
        };
        input.locks = crate::all_parameters(false)
            .into_iter()
            .filter(|p| ![Parameter::Algorithm, mul].contains(p))
            .collect();
        let mut seed = input.initial.clone();
        mul.set(&mut seed, 13);
        input.seeds.push(seed.clone());
        let mut algorithms = [false; 8];
        let mut saw_seed = false;
        optimize(
            &input,
            context,
            &|| false,
            |patches| {
                patches
                    .iter()
                    .map(|patch| {
                        algorithms[usize::from(patch.channel.algorithm.get())] = true;
                        saw_seed |= *patch == seed;
                        Ok(scalar_loss(
                            (f64::from(mul.get(patch)) - 13.0).abs()
                                + f64::from(patch.channel.algorithm.get()),
                        ))
                    })
                    .collect()
            },
            None,
            Vec::new(),
            |_| {},
        )
        .unwrap();
        assert!(algorithms.into_iter().all(|seen| seen));
        assert!(saw_seed);
    }
    #[test]
    fn special_frequency_locks_survive_adversarial_seeds_and_automatic_mode_proposals() {
        let (mut input, mut context) = fixture();
        input.initial.channel3_special = Some([discofm_chip::Frequency::new(777, 3).unwrap(); 3]);
        input.audition.channel = discofm_chip::Channel::THREE;
        context.audition = input.audition.clone();
        let locked = Parameter::SpecialFnum { index: 0 };
        input.locks.insert(locked);
        input.seeds.push(Instrument::default());
        optimize(
            &input,
            context,
            &|| false,
            |patches| {
                patches
                    .iter()
                    .map(|patch| {
                        assert!(patch.channel3_special.is_some());
                        assert_eq!(locked.get(patch), 777);
                        Ok(scalar_loss(1.0))
                    })
                    .collect()
            },
            None,
            vec![
                crate::tracking::Peak {
                    hz: 528.0,
                    amplitude: 1.0,
                },
                crate::tracking::Peak {
                    hz: 1049.0,
                    amplitude: 0.5,
                },
            ],
            |_| {},
        )
        .unwrap();
    }
    #[test]
    fn muted_locked_carriers_do_not_consume_root_or_partial_seeds() {
        let (mut input, context) = fixture();
        Parameter::Algorithm.set(&mut input.initial, 7);
        for index in [0, 3] {
            let p = Parameter::Operator {
                index,
                field: OperatorField::Tl,
            };
            p.set(&mut input.initial, 127);
            input.locks.insert(p);
        }
        let search = Search {
            input: &input,
            evaluate: |_: &[Instrument]| Ok(Vec::new()),
            screen: None,
            screen_cache: BTreeMap::new(),
            progress: |_| {},
            cache: BTreeMap::new(),
            speculative: BTreeMap::new(),
            candidates: Vec::new(),
            families: std::array::from_fn(|_| None),
            history: History::new(),
            initial_loss: None,
            evaluations: 0,
            renders: 0,
            generation: 0,
            current: 0.0,
            stage: FitStage::Coarse,
            start: Instant::now(),
            context,
            anchors: vec![
                crate::tracking::Peak {
                    hz: 1049.0,
                    amplitude: 1.0,
                },
                crate::tracking::Peak {
                    hz: 440.0,
                    amplitude: 0.5,
                },
                crate::tracking::Peak {
                    hz: 2099.0,
                    amplitude: 0.3,
                },
            ],
        };
        let mut patch = input.initial.clone();
        Parameter::Channel3Mode.set(&mut patch, 1);
        let original = patch.channel3_special.unwrap()[0];
        search.seed_partials(&mut patch, &search.active(FitStage::Refine));
        let frequencies = patch.channel3_special.unwrap();
        assert_eq!(frequencies[0], original);
        for (frequency, hz) in [(frequencies[1], 1049.0), (frequencies[2], 440.0)] {
            assert!(
                (discofm_engine::frequency_hz(frequency, input.audition.clock) - hz).abs() < 1.0
            );
        }
        assert_eq!(patch.operators[0].tl.get(), 127);
        assert_eq!(patch.operators[3].tl.get(), 127);
        assert_eq!(patch.operators[1].tl.get(), 0);
        assert!(patch.operators[2].tl.get() < 127);
    }
    #[test]
    fn coherent_levels_respect_locks_and_reserve_envelopes_for_the_opposite_topology() {
        let (mut input, context) = fixture();
        input.config.population = 32;
        let locked = Parameter::Operator {
            index: 0,
            field: OperatorField::Tl,
        };
        input.locks.insert(locked);
        let mut normal = input.initial.clone();
        Parameter::Algorithm.set(&mut normal, 7);
        for index in 0..4 {
            for (field, value) in [(OperatorField::Tl, 28), (OperatorField::D1r, 8)] {
                Parameter::Operator { index, field }.set(&mut normal, value);
            }
        }
        locked.set(&mut input.initial, 28);
        let mut special = normal.clone();
        Parameter::Channel3Mode.set(&mut special, 1);
        let mut levels = [false; 2];
        let mut envelopes = [false; 2];
        let evaluate = |patches: &[Instrument]| {
            Ok(patches
                .iter()
                .map(|patch| {
                    assert_eq!(locked.get(patch), 28);
                    let family = usize::from(patch.channel3_special.is_some());
                    let shared_level = patch.operators[1..].iter().all(|op| op.tl.get() == 0);
                    let shared_envelope = patch.operators.iter().all(|op| op.d1r.get() == 12);
                    levels[family] |= shared_level;
                    envelopes[family] |= shared_envelope;
                    scalar_loss(if family == 1 && shared_envelope {
                        0.0
                    } else {
                        10.0 + family as f64 - f64::from(shared_level)
                    })
                })
                .collect())
        };
        let mut search = Search {
            input: &input,
            evaluate,
            screen: None,
            screen_cache: BTreeMap::new(),
            progress: |_| {},
            cache: BTreeMap::new(),
            speculative: BTreeMap::new(),
            candidates: Vec::new(),
            families: std::array::from_fn(|_| None),
            history: History::new(),
            initial_loss: None,
            evaluations: 0,
            renders: 0,
            generation: 0,
            current: 0.0,
            stage: FitStage::Envelopes,
            start: Instant::now(),
            context,
            anchors: Vec::new(),
        };
        search.score_many(vec![normal, special], &|| false).unwrap();
        search.carrier_envelopes(34, false, &|| false).unwrap();
        assert_eq!(search.candidates[0].loss.total, 0.0);
        assert!(search.candidates[0].instrument.channel3_special.is_some());
        assert!(search.evaluations <= 34);
        drop(search);
        assert_eq!(levels, [true; 2]);
        assert_eq!(envelopes, [true; 2]);
    }
    #[test]
    fn speculative_lookahead_fills_idle_workers_without_changing_any_decision() {
        let (mut input, context) = fixture();
        input.config.evaluations = 400;
        // A rugged, deterministic objective so leaders change during refinement.
        let loss = |patch: &Instrument| {
            let mut hash = 0xcbf29ce484222325_u64;
            for value in key(patch) {
                hash = (hash ^ u64::from(value)).wrapping_mul(0x100000001b3);
            }
            let target = crate::all_parameters(false)
                .iter()
                .map(|p| (f64::from(p.get(patch)) / f64::from(p.maximum()) - 0.4).abs())
                .sum::<f64>();
            scalar_loss(target + (hash % 1000) as f64 * 1e-4)
        };
        let mut runs = Vec::new();
        for workers in [1, 3, 8] {
            input.config.workers = workers;
            let mut sent = 0;
            let outcome = optimize(
                &input,
                context.clone(),
                &|| false,
                |patches| {
                    sent += patches.len();
                    Ok(patches.iter().map(loss).collect())
                },
                None,
                Vec::new(),
                |_| {},
            )
            .unwrap();
            runs.push((outcome, sent));
        }
        let (serial, serial_sent) = &runs[0];
        assert_eq!(*serial_sent, serial.renders);
        for (parallel, _) in &runs[1..] {
            assert_eq!(serial.candidates, parallel.candidates);
            assert_eq!(serial.history, parallel.history);
            assert_eq!(serial.renders, parallel.renders);
            assert_eq!(serial.evaluations, parallel.evaluations);
            assert_eq!(serial.generations, parallel.generations);
        }
        assert!(
            runs[2].1 > serial.renders,
            "idle workers received lookahead"
        );
    }
    #[test]
    fn cancellation_after_a_finished_batch_keeps_completed_scores_and_valid_cache_counts() {
        let (mut input, context) = fixture();
        input.config.workers = 4;
        let stopped = std::cell::Cell::new(false);
        let finished = std::cell::Cell::new(0);
        let result = optimize(
            &input,
            context,
            &|| stopped.get(),
            |patches| {
                finished.set(finished.get() + patches.len());
                if patches.len() > 1 {
                    stopped.set(true);
                }
                Ok(patches.iter().map(|_| scalar_loss(1.0)).collect())
            },
            None,
            Vec::new(),
            |_| {},
        )
        .unwrap();
        assert_eq!(result.status, FitStatus::Cancelled);
        // Unrequested lookahead scores are sent to workers but never counted.
        assert!(result.renders >= 1 && result.renders <= finished.get());
        assert_eq!(result.performance.cache_entries, result.renders);
        result.validate().unwrap();
    }
}
