//! Sound cues, fixture only: the i18n table's sound-set test reads Cue and
//! SoundSet, and the General pane plays the dictation-start tone.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SoundSet {
    #[default]
    Standard,
}

impl SoundSet {
    pub const ALL: [Self; 0] = [];

    pub fn name(self) -> &'static str {
        "Sounds"
    }

    pub fn character(self) -> &'static str {
        "Clear"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cue {
    DictationStart,
}

impl Cue {
    pub const ALL: [Self; 0] = [];

    pub fn name(self) -> &'static str {
        "Dictation start"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    DictationStart,
}

pub(crate) fn play(_tone: Tone) {}
