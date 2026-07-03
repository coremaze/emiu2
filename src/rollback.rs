//! The platform side of IR rollback: owns the snapshot and executes the
//! transport's directives against the machine.
//!
//! Rollback is a property of the link, not of the emulated hardware —
//! a local or disconnected link never creates a driver, and the machine
//! itself (see `miuchiz::st2205u`) knows nothing about any of this. Run
//! loops call [`RollbackDriver::run`] once per pacing quantum and
//! re-anchor their wall-clock pacing whenever it reports that the
//! machine state jumped.

use crate::ir::{IrRollbackControl, RollbackDirective};
use crate::miuchiz::Handheld;

pub struct RollbackDriver {
    control: Box<dyn IrRollbackControl>,
    /// The serialized machine at the last receiver-on point.
    snapshot: Vec<u8>,
    have_snapshot: bool,
    /// Retired snapshot buffer, reused to avoid reallocating ~2MiB per
    /// snapshot.
    spare: Vec<u8>,
}

impl RollbackDriver {
    pub fn new(control: Box<dyn IrRollbackControl>) -> Self {
        Self {
            control,
            snapshot: Vec::new(),
            have_snapshot: false,
            spare: Vec::new(),
        }
    }

    /// Executes at most one directive. Returns true when the machine
    /// state jumped (a rollback happened) and the caller must re-anchor
    /// its pacing so the machine resumes at 1x from the restored point.
    pub fn run(&mut self, handheld: &mut Handheld) -> bool {
        match self.control.poll(handheld.mcu.core.oscillator_cycles()) {
            RollbackDirective::Continue => false,
            RollbackDirective::TakeSnapshot => {
                let buf = std::mem::take(&mut self.spare);
                let filled = handheld.snapshot_reusing(buf);
                self.spare = std::mem::replace(&mut self.snapshot, filled);
                self.have_snapshot = true;
                self.control
                    .snapshot_taken(handheld.mcu.core.oscillator_cycles());
                false
            }
            RollbackDirective::RollBack => {
                if !self.have_snapshot {
                    return false;
                }
                let abandoned = handheld.mcu.core.oscillator_cycles();
                match handheld.restore(&self.snapshot) {
                    Ok(()) => {
                        self.control
                            .rolled_back(handheld.mcu.core.oscillator_cycles(), abandoned);
                        true
                    }
                    Err(why) => {
                        eprintln!("IR rollback failed to restore snapshot: {why}");
                        false
                    }
                }
            }
        }
    }
}
