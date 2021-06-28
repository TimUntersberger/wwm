use bitflags::bitflags;

bitflags! {
    #[derive(Default)]
    #[allow(dead_code)]
    pub struct Modifier: u32 {
        const CONTROL = 0b00000001;
        const WIN = 0b00000010;
        const SHIFT = 0b00000100;
        const LALT = 0b00001000;
        const RALT = 0b00010000;
    }
}
