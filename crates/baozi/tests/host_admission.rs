mod support;

use baozi::{
    CapabilityStatus, FormatCapability, FormatDescriptor, FormatEncoding, FormatId, Importer,
    ReadConfidence,
};
use baozi_import::{
    DetectionOutcome, DetectionResult, DetectionScope, FormatImporter, ParseResult, ParseScope,
    Probe, ReadHint, SceneDraft,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

const FORMAT_ID: &str = "org.example.host-admission";
const THREAD_COUNT: usize = 4;
const SYNCHRONIZATION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
struct GateState {
    in_flight: usize,
    peak: usize,
}

#[derive(Debug)]
struct HostGate {
    capacity: usize,
    state: Mutex<GateState>,
    available: Condvar,
}

impl HostGate {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "host gate capacity must be nonzero");
        Self {
            capacity,
            state: Mutex::new(GateState::default()),
            available: Condvar::new(),
        }
    }

    fn enter(&self) -> HostPermit<'_> {
        let mut state = self.state.lock().expect("host gate state is not poisoned");
        while state.in_flight == self.capacity {
            state = self
                .available
                .wait(state)
                .expect("host gate state is not poisoned");
        }
        state.in_flight += 1;
        state.peak = state.peak.max(state.in_flight);
        drop(state);
        HostPermit { gate: self }
    }

    fn snapshot(&self) -> GateState {
        let state = self.state.lock().expect("host gate state is not poisoned");
        GateState {
            in_flight: state.in_flight,
            peak: state.peak,
        }
    }
}

struct HostPermit<'a> {
    gate: &'a HostGate,
}

impl Drop for HostPermit<'_> {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.in_flight = state
            .in_flight
            .checked_sub(1)
            .expect("host permit is released exactly once");
        drop(state);
        self.gate.available.notify_one();
    }
}

struct ParserProbe {
    current: AtomicUsize,
    peak: AtomicUsize,
    entered: Sender<()>,
    releases: Mutex<Receiver<()>>,
}

impl ParserProbe {
    fn new(entered: Sender<()>, releases: Receiver<()>) -> Self {
        Self {
            current: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            entered,
            releases: Mutex::new(releases),
        }
    }

    fn enter(&self) -> ParserPermit<'_> {
        let current = self.current.fetch_add(1, Ordering::AcqRel) + 1;
        self.peak.fetch_max(current, Ordering::AcqRel);
        let permit = ParserPermit { probe: self };
        self.entered
            .send(())
            .expect("host synchronization receiver remains alive");
        self.releases
            .lock()
            .expect("parser release receiver is not poisoned")
            .recv()
            .expect("host releases every admitted parser");
        permit
    }

    fn current(&self) -> usize {
        self.current.load(Ordering::Acquire)
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }
}

struct ParserPermit<'a> {
    probe: &'a ParserProbe,
}

impl Drop for ParserPermit<'_> {
    fn drop(&mut self) {
        self.probe.current.fetch_sub(1, Ordering::AcqRel);
    }
}

struct AdmissionProbeImporter {
    probe: Arc<ParserProbe>,
}

impl FormatImporter for AdmissionProbeImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new(FORMAT_ID).expect("qualified host-admission format id"),
            "Host admission probe",
        )
        .extensions(["host-gate"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Unknown)])
        .build()
        .expect("valid host-admission descriptor")
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let _permit = self.probe.enter();
        support::submit_minimal_scene(scope)
    }
}

#[test]
fn external_host_gate_is_the_only_concurrent_session_admission_authority() {
    // This characterizes an already runtime-neutral production path: the missing U3 evidence was
    // an acceptance test, not a production gate that needed to be added to Baozi.
    for capacity in [1_usize, 2] {
        run_admission_case(capacity);
    }
}

fn run_admission_case(capacity: usize) {
    let gate = Arc::new(HostGate::new(capacity));
    let (entered_sender, entered_receiver) = channel();
    let (release_sender, release_receiver) = channel();
    let probe = Arc::new(ParserProbe::new(entered_sender, release_receiver));
    let mut builder = Importer::builder();
    builder
        .register(AdmissionProbeImporter {
            probe: Arc::clone(&probe),
        })
        .expect("host-admission importer must register");
    let importer = Arc::new(builder.build().expect("registry must freeze"));

    let mut workers = Vec::with_capacity(THREAD_COUNT);
    for index in 0..THREAD_COUNT {
        let gate = Arc::clone(&gate);
        let importer = Arc::clone(&importer);
        workers.push(thread::spawn(move || {
            let _permit = gate.enter();
            importer
                .read_bytes(format!("asset-{index}.host-gate"), b"fixture")
                .expect("admitted import must succeed");
        }));
    }

    for wave in 0..(THREAD_COUNT / capacity) {
        for participant in 0..capacity {
            entered_receiver
                .recv_timeout(SYNCHRONIZATION_TIMEOUT)
                .unwrap_or_else(|error| {
                    panic!(
                        "capacity {capacity} wave {wave} participant {participant} did not enter: {error}"
                    )
                });
        }
        assert_eq!(
            probe.current(),
            capacity,
            "the parser overlap must be controlled only by the host gate"
        );
        for _ in 0..capacity {
            release_sender
                .send(())
                .expect("release every parser in the admitted wave");
        }
    }

    for worker in workers {
        worker.join().expect("host worker must not panic");
    }

    let gate_state = gate.snapshot();
    assert_eq!(gate_state.in_flight, 0);
    assert_eq!(gate_state.peak, capacity);
    assert_eq!(probe.current(), 0);
    assert_eq!(probe.peak(), capacity);
}
