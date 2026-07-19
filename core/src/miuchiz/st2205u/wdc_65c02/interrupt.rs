pub trait HandlesInterrupt {
    fn set_interrupted(&mut self, interrupted: bool);
    fn interrupted(&self) -> bool;
}
