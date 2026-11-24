use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

pub const WAVEFORM_BARS: usize = 36;

pub struct VoiceRecorder {
    child: Child,
    stop_flag: Arc<AtomicBool>,
    levels: Arc<Mutex<VecDeque<f32>>>,
    pcm_bytes: Arc<Mutex<Vec<u8>>>,
    reader_handle: Option<JoinHandle<()>>,
}

impl VoiceRecorder {
    pub fn start() -> Result<Self, String> {
        let mut child = Command::new("pw-record")
            .args([
                "--raw",
                "--format",
                "s16",
                "--rate",
                "16000",
                "--channels",
                "1",
                "-",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Impossible de lancer pw-record : {e}"))?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Flux audio pw-record indisponible".to_string())?;

        let levels = Arc::new(Mutex::new(VecDeque::from(vec![0.08_f32; WAVEFORM_BARS])));
        let pcm_bytes = Arc::new(Mutex::new(Vec::with_capacity(16000 * 2 * 10)));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let levels_bg = levels.clone();
        let pcm_bg = pcm_bytes.clone();
        let stop_bg = stop_flag.clone();

        let reader_handle = thread::spawn(move || {
            let mut buf = [0u8; 1024]; // ~32ms at 16 kHz mono s16le
            while !stop_bg.load(Ordering::Relaxed) {
                match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = &buf[..n];
                        if let Ok(mut pcm) = pcm_bg.lock() {
                            pcm.extend_from_slice(chunk);
                        }
                        let level = compute_rms_level(chunk);
                        if let Ok(mut q) = levels_bg.lock() {
                            if q.len() >= WAVEFORM_BARS {
                                q.pop_front();
                            }
                            q.push_back(level);
                        }
                    }
                }
            }
        });

        Ok(Self {
            child,
            stop_flag,
            levels,
            pcm_bytes,
            reader_handle: Some(reader_handle),
        })
    }

    pub fn levels(&self) -> Arc<Mutex<VecDeque<f32>>> {
        self.levels.clone()
    }

    pub fn stop_and_write_wav(mut self) -> Result<PathBuf, String> {
        self.stop_child();
        let pcm = self
            .pcm_bytes
            .lock()
            .map_err(|_| "Verrou audio corrompu".to_string())?
            .clone();

        if pcm.len() < 3200 {
            return Err("Enregistrement trop court".to_string());
        }

        let wav = write_wav_bytes(&pcm);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "ai-overview-voice-{}-{ts}.wav",
            std::process::id()
        ));
        std::fs::write(&path, wav).map_err(|e| format!("Écriture WAV échouée : {e}"))?;
        Ok(path)
    }

    pub fn cancel(mut self) {
        self.stop_child();
    }

    fn stop_child(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.join();
        }
    }
}

fn compute_rms_level(pcm_s16le: &[u8]) -> f32 {
    let sample_count = pcm_s16le.len() / 2;
    if sample_count == 0 {
        return 0.08;
    }
    let mut sum_sq = 0.0_f64;
    for pair in pcm_s16le.chunks_exact(2) {
        let sample = i16::from_le_bytes([pair[0], pair[1]]) as f64 / 32768.0;
        sum_sq += sample * sample;
    }
    let rms = (sum_sq / sample_count as f64).sqrt() as f32;
    (rms * 7.0).sqrt().clamp(0.08, 1.0)
}

pub fn write_wav_bytes(pcm: &[u8]) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let riff_len = 36_u32.saturating_add(data_len);
    let sample_rate: u32 = 16000;
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate: u32 = sample_rate * (channels as u32) * (bits_per_sample as u32 / 8);
    let block_align: u16 = channels * (bits_per_sample / 8);

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16_u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM format
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

#[derive(Deserialize)]
struct HandyJsonOutput {
    text: String,
}

pub fn parse_handy_json_stdout(stdout: &str) -> Result<String, String> {
    for line in stdout.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && let Ok(parsed) = serde_json::from_str::<HandyJsonOutput>(trimmed)
        {
            return Ok(parsed.text.trim().to_string());
        }
    }
    Err("Réponse JSON Handy introuvable".to_string())
}

pub fn transcribe_with_handy(wav_path: &Path) -> Result<String, String> {
    let output = Command::new("handy")
        .arg("--transcribe-file")
        .arg(wav_path)
        .arg("--json")
        .output();

    let _ = std::fs::remove_file(wav_path);

    let output = output.map_err(|e| format!("Impossible de lancer handy : {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Handy a échoué : {}", err.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_handy_json_stdout(&stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_valid_16khz_mono() {
        let pcm = vec![0u8; 320];
        let wav = write_wav_bytes(&pcm);
        assert_eq!(wav.len(), 44 + 320);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]), 16000);
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 320);
    }

    #[test]
    fn parses_handy_json_stdout() {
        let sample = "some info log\n{\"audio_secs\":1.2,\"text\":\"  Bonjour le monde  \"}\n";
        assert_eq!(
            parse_handy_json_stdout(sample).expect("parsed"),
            "Bonjour le monde"
        );
    }
}
