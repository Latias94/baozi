/// One linear attempted-and-consumed work step paired with one startup account share.
pub(crate) struct StartupAccountShareStep<D> {
    charges: [(D, u64); 2],
}

impl<D> StartupAccountShareStep<D> {
    pub(crate) fn into_charges(self) -> [(D, u64); 2] {
        self.charges
    }
}

/// The exact linear work steps consumed by retained startup's account shares.
///
/// The fixed step count is defined only by this envelope. Current-state preflight and execution
/// both use these stored steps, so their work requirements stay aligned.
pub(crate) struct StartupAccountShareEnvelope<D, const N: usize = 4> {
    steps: [StartupAccountShareStep<D>; N],
}

impl<D, const N: usize> StartupAccountShareEnvelope<D, N> {
    pub(crate) fn into_steps(self) -> [StartupAccountShareStep<D>; N] {
        self.steps
    }
}

impl<D: Copy> StartupAccountShareEnvelope<D, 4> {
    pub(crate) const fn four_identical(charges: [(D, u64); 2]) -> Self {
        Self {
            steps: [
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
            ],
        }
    }

    pub(crate) fn work_specs(&self) -> [(D, u64); 8] {
        [
            self.steps[0].charges[0],
            self.steps[0].charges[1],
            self.steps[1].charges[0],
            self.steps[1].charges[1],
            self.steps[2].charges[0],
            self.steps[2].charges[1],
            self.steps[3].charges[0],
            self.steps[3].charges[1],
        ]
    }
}

impl<D: Copy> StartupAccountShareEnvelope<D, 3> {
    pub(crate) const fn three_identical(charges: [(D, u64); 2]) -> Self {
        Self {
            steps: [
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
                StartupAccountShareStep { charges },
            ],
        }
    }

    pub(crate) fn work_specs(&self) -> [(D, u64); 6] {
        [
            self.steps[0].charges[0],
            self.steps[0].charges[1],
            self.steps[1].charges[0],
            self.steps[1].charges[1],
            self.steps[2].charges[0],
            self.steps[2].charges[1],
        ]
    }
}
