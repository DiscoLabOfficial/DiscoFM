//! Instrument undo/redo. A drag or a typed value is one step: a step opens on
//! the first frame that changes the instrument and closes once no pointer
//! button is down and no widget holds keyboard focus.
use discofm_chip::Instrument;

const LIMIT: usize = 200;

#[derive(Default)]
pub(crate) struct Undo {
    undo: Vec<Instrument>,
    redo: Vec<Instrument>,
    open: bool,
}
impl Undo {
    /// Once per frame, with the instrument before and after the frame's edits.
    pub fn track(&mut self, before: &Instrument, after: &Instrument, settled: bool) {
        if before != after && !self.open {
            self.push(before.clone());
            self.open = true;
        }
        if settled {
            self.open = false;
        }
    }
    /// A replacement made outside the editor widgets (paste, import, library).
    pub fn record(&mut self, before: Instrument) {
        self.open = false;
        self.push(before);
    }
    fn push(&mut self, before: Instrument) {
        if self.undo.last() != Some(&before) {
            self.undo.push(before);
            if self.undo.len() > LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }
    pub fn undo(&mut self, current: &mut Instrument) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(current, previous));
        self.open = false;
        true
    }
    pub fn redo(&mut self, current: &mut Instrument) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(current, next));
        self.open = false;
        true
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use discofm_chip::Value;

    fn with_tl(tl: u8) -> Instrument {
        let mut instrument = Instrument::default();
        instrument.operators[0].tl = Value::new(tl).unwrap();
        instrument
    }

    #[test]
    fn a_drag_is_one_step_and_new_edits_clear_redo() {
        let mut undo = Undo::default();
        let mut current = with_tl(10);
        // A drag across three frames, released on the last one.
        for (tl, settled) in [(11, false), (12, false), (13, true)] {
            let before = current.clone();
            current = with_tl(tl);
            undo.track(&before, &current, settled);
        }
        let before = current.clone();
        current = with_tl(20);
        undo.track(&before, &current, true);
        assert!(undo.undo(&mut current));
        assert_eq!(current, with_tl(13));
        assert!(undo.undo(&mut current));
        assert_eq!(current, with_tl(10));
        assert!(!undo.undo(&mut current));
        assert!(undo.redo(&mut current));
        assert_eq!(current, with_tl(13));
        undo.record(current.clone());
        current = with_tl(30);
        assert!(!undo.can_redo());
        assert!(undo.undo(&mut current));
        assert_eq!(current, with_tl(13));
    }
}
