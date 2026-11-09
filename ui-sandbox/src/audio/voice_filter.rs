//! Audio files, fixture only.

/// Reads a mono WAV; the sandbox has no recordings.
pub(crate) fn read_wav_mono_f32(_path: &std::path::Path) -> Result<(Vec<f32>, u32), String> {
    Err("the sandbox has no recordings".into())
}
