//! Avisos sonoros ao entrar no âmbar e no vermelho.
//!
//! Os sons são sintetizados aqui (tons senoidais em WAV PCM 16-bit mono), então
//! não há arquivos de áudio no repositório.

use std::sync::OnceLock;

use crate::fmt::Level;

const SAMPLE_RATE: u32 = 22_050;
const FADE_MS: u32 = 5;
/// Tom `(frequência em Hz, duração em ms)`; frequência 0 é silêncio.
type Tone = (f64, u32);

const AMBER: &[Tone] = &[(660.0, 120), (0.0, 80), (880.0, 160)];
const RED: &[Tone] = &[
    (1000.0, 150),
    (1400.0, 150),
    (0.0, 70),
    (1000.0, 150),
    (1400.0, 150),
    (0.0, 70),
    (1000.0, 150),
    (1400.0, 150),
];

/// Qual aviso tocar ao passar de `prev` para `now`: só quando o nível sobe e
/// só o do nível novo (Low→High toca apenas o vermelho).
pub fn on_reading(prev: Level, now: Level) -> Option<Level> {
    (now > prev && now != Level::Low).then_some(now)
}

/// WAV pronto para `PlaySound` (cabeçalho RIFF + amostras).
pub fn wav(tones: &[Tone], volume: f64) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for &(freq, ms) in tones {
        let n = (SAMPLE_RATE as u64 * ms as u64 / 1000) as usize;
        let fade = (SAMPLE_RATE * FADE_MS / 1000) as usize;
        for i in 0..n {
            if freq <= 0.0 {
                samples.push(0);
                continue;
            }
            // Fade nas pontas para o tom não estalar.
            let edge = i.min(n - 1 - i);
            let env = if edge < fade {
                edge as f64 / fade as f64
            } else {
                1.0
            };
            let t = i as f64 / SAMPLE_RATE as f64;
            let v = (2.0 * std::f64::consts::PI * freq * t).sin() * env * volume;
            samples.push((v * i16::MAX as f64) as i16);
        }
    }
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // bytes/s
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// O som do nível, com vida `'static`: `SND_ASYNC` toca de dentro do buffer
/// depois que `PlaySound` retorna. `Low` não tem som.
pub fn sound(level: Level) -> Option<&'static [u8]> {
    static AMBER_WAV: OnceLock<Vec<u8>> = OnceLock::new();
    static RED_WAV: OnceLock<Vec<u8>> = OnceLock::new();
    match level {
        Level::Low => None,
        Level::Mid => Some(AMBER_WAV.get_or_init(|| wav(AMBER, 0.45))),
        Level::High => Some(RED_WAV.get_or_init(|| wav(RED, 0.9))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rises_only() {
        use Level::*;
        assert_eq!(on_reading(Low, Mid), Some(Mid));
        assert_eq!(on_reading(Mid, High), Some(High));
        assert_eq!(on_reading(Low, High), Some(High));
        assert_eq!(on_reading(Low, Low), None);
        assert_eq!(on_reading(Mid, Mid), None);
        assert_eq!(on_reading(High, High), None);
        assert_eq!(on_reading(High, Mid), None);
        assert_eq!(on_reading(High, Low), None);
        assert_eq!(on_reading(Mid, Low), None);
    }

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
    }

    #[test]
    fn wav_header_is_consistent() {
        let w = wav(AMBER, 0.5);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(u32_at(&w, 4) as usize, w.len() - 8);
        assert_eq!(u32_at(&w, 24), SAMPLE_RATE);
        assert_eq!(u32_at(&w, 40) as usize, w.len() - 44);
        let ms: u32 = AMBER.iter().map(|t| t.1).sum();
        let expected = (SAMPLE_RATE as u64 * ms as u64 / 1000) as usize * 2;
        assert!(w.len() - 44 >= expected - 8 && w.len() - 44 <= expected + 8);
    }

    #[test]
    fn red_differs_from_amber_and_is_longer() {
        let a = sound(Level::Mid).unwrap();
        let r = sound(Level::High).unwrap();
        assert_ne!(a, r);
        assert!(r.len() > a.len());
        assert!(sound(Level::Low).is_none());
    }

    #[test]
    #[ignore = "toca de verdade: cargo test plays_both_sounds -- --ignored"]
    fn plays_both_sounds() {
        // Síncrono aqui só para o teste esperar o fim de cada som.
        for (level, wait) in [(Level::Mid, 1500), (Level::High, 2500)] {
            crate::win::play_wav(sound(level).unwrap());
            std::thread::sleep(std::time::Duration::from_millis(wait));
        }
    }
}
