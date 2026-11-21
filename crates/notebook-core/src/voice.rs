use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr::NonNull;

#[cfg(target_os = "android")]
mod ffi {
    use super::*;

    #[repr(C)]
    pub struct VoicevoxOnnxruntime { _private: [u8; 0] }
    #[repr(C)]
    pub struct OpenJtalkRc { _private: [u8; 0] }
    #[repr(C)]
    pub struct VoicevoxSynthesizer { _private: [u8; 0] }
    #[repr(C)]
    pub struct VoicevoxVoiceModelFile { _private: [u8; 0] }

    pub type VoicevoxResultCode = i32;
    pub type VoicevoxStyleId = u32;
    pub type VoicevoxAccelerationMode = u32;

    #[repr(C)]
    pub struct VoicevoxLoadOnnxruntimeOptions {
        pub filename: *const std::os::raw::c_char,
    }

    #[repr(C)]
    pub struct VoicevoxInitializeOptions {
        pub acceleration_mode: VoicevoxAccelerationMode,
        pub cpu_num_threads: u16,
    }

    #[repr(C)]
    pub struct VoicevoxLoadVoiceModelOptions {
        pub on_existing: i32,
    }

    #[repr(C)]
    pub struct VoicevoxTtsOptions {
        pub enable_interrogative_upspeak: bool,
    }

    extern "C" {
        pub fn voicevox_onnxruntime_load_once(
            options: VoicevoxLoadOnnxruntimeOptions,
            out: *mut *const VoicevoxOnnxruntime,
        ) -> VoicevoxResultCode;

        pub fn voicevox_open_jtalk_rc_new(
            dic_dir: *const std::os::raw::c_char,
            out: *mut *mut OpenJtalkRc,
        ) -> VoicevoxResultCode;

        pub fn voicevox_make_default_initialize_options() -> VoicevoxInitializeOptions;

        pub fn voicevox_synthesizer_new(
            onnxruntime: *const VoicevoxOnnxruntime,
            open_jtalk: *const OpenJtalkRc,
            options: VoicevoxInitializeOptions,
            out: *mut *mut VoicevoxSynthesizer,
        ) -> VoicevoxResultCode;

        pub fn voicevox_voice_model_file_open(
            path: *const std::os::raw::c_char,
            out: *mut *mut VoicevoxVoiceModelFile,
        ) -> VoicevoxResultCode;

        pub fn voicevox_make_default_load_voice_model_options() -> VoicevoxLoadVoiceModelOptions;

        pub fn voicevox_synthesizer_load_voice_model(
            synthesizer: *const VoicevoxSynthesizer,
            voice_model_file: *const VoicevoxVoiceModelFile,
            options: VoicevoxLoadVoiceModelOptions,
        ) -> VoicevoxResultCode;

        pub fn voicevox_make_default_tts_options() -> VoicevoxTtsOptions;

        pub fn voicevox_synthesizer_tts(
            synthesizer: *const VoicevoxSynthesizer,
            text: *const std::os::raw::c_char,
            style_id: VoicevoxStyleId,
            options: VoicevoxTtsOptions,
            out_len: *mut usize,
            out_wav: *mut *mut u8,
        ) -> VoicevoxResultCode;

        pub fn voicevox_synthesizer_delete(synthesizer: *mut VoicevoxSynthesizer);
        pub fn voicevox_open_jtalk_rc_delete(open_jtalk: *mut OpenJtalkRc);
        pub fn voicevox_voice_model_file_delete(voice_model_file: *mut VoicevoxVoiceModelFile);
        pub fn voicevox_wav_free(wav: *mut u8);

        pub fn voicevox_error_result_to_message(code: VoicevoxResultCode) -> *const std::os::raw::c_char;
    }

    pub const ACCELERATION_MODE_CPU: VoicevoxAccelerationMode = 1;

    #[link(name = "voicevox_core")]
    extern "C" {}
}

#[cfg(not(target_os = "android"))]
mod ffi {
    use super::*;

    pub type VoicevoxResultCode = i32;
    pub type VoicevoxStyleId = u32;
    pub type VoicevoxAccelerationMode = u32;

    pub struct VoicevoxOnnxruntime { _private: [u8; 0] }
    pub struct OpenJtalkRc { _private: [u8; 0] }
    pub struct VoicevoxSynthesizer { _private: [u8; 0] }
    pub struct VoicevoxVoiceModelFile { _private: [u8; 0] }

    #[repr(C)]
    pub struct VoicevoxLoadOnnxruntimeOptions {
        pub filename: *const std::os::raw::c_char,
    }

    #[repr(C)]
    pub struct VoicevoxInitializeOptions {
        pub acceleration_mode: VoicevoxAccelerationMode,
        pub cpu_num_threads: u16,
    }

    #[repr(C)]
    pub struct VoicevoxLoadVoiceModelOptions {
        pub on_existing: i32,
    }

    #[repr(C)]
    pub struct VoicevoxTtsOptions {
        pub enable_interrogative_upspeak: bool,
    }

    pub const ACCELERATION_MODE_CPU: VoicevoxAccelerationMode = 1;

    pub fn voicevox_onnxruntime_load_once(
        _options: VoicevoxLoadOnnxruntimeOptions,
        _out: *mut *const VoicevoxOnnxruntime,
    ) -> VoicevoxResultCode {
        -1
    }

    pub fn voicevox_open_jtalk_rc_new(
        _dic_dir: *const std::os::raw::c_char,
        _out: *mut *mut OpenJtalkRc,
    ) -> VoicevoxResultCode {
        -1
    }

    pub fn voicevox_make_default_initialize_options() -> VoicevoxInitializeOptions {
        VoicevoxInitializeOptions {
            acceleration_mode: ACCELERATION_MODE_CPU,
            cpu_num_threads: 1,
        }
    }

    pub fn voicevox_synthesizer_new(
        _onnxruntime: *const VoicevoxOnnxruntime,
        _open_jtalk: *const OpenJtalkRc,
        _options: VoicevoxInitializeOptions,
        _out: *mut *mut VoicevoxSynthesizer,
    ) -> VoicevoxResultCode {
        -1
    }

    pub fn voicevox_voice_model_file_open(
        _path: *const std::os::raw::c_char,
        _out: *mut *mut VoicevoxVoiceModelFile,
    ) -> VoicevoxResultCode {
        -1
    }

    pub fn voicevox_make_default_load_voice_model_options() -> VoicevoxLoadVoiceModelOptions {
        VoicevoxLoadVoiceModelOptions { on_existing: 0 }
    }

    pub fn voicevox_synthesizer_load_voice_model(
        _synthesizer: *const VoicevoxSynthesizer,
        _voice_model_file: *const VoicevoxVoiceModelFile,
        _options: VoicevoxLoadVoiceModelOptions,
    ) -> VoicevoxResultCode {
        -1
    }

    pub fn voicevox_make_default_tts_options() -> VoicevoxTtsOptions {
        VoicevoxTtsOptions { enable_interrogative_upspeak: true }
    }

    pub fn voicevox_synthesizer_tts(
        _synthesizer: *const VoicevoxSynthesizer,
        _text: *const std::os::raw::c_char,
        _style_id: VoicevoxStyleId,
        _options: VoicevoxTtsOptions,
        _out_len: *mut usize,
        _out_wav: *mut *mut u8,
    ) -> VoicevoxResultCode {
        -1
    }

    pub fn voicevox_synthesizer_delete(_synthesizer: *mut VoicevoxSynthesizer) {}
    pub fn voicevox_open_jtalk_rc_delete(_open_jtalk: *mut OpenJtalkRc) {}
    pub fn voicevox_voice_model_file_delete(_voice_model_file: *mut VoicevoxVoiceModelFile) {}
    pub fn voicevox_wav_free(_wav: *mut u8) {}

    pub unsafe fn voicevox_error_result_to_message(code: VoicevoxResultCode) -> *const std::os::raw::c_char {
        match code {
            -1 => b"not available on this platform\0".as_ptr() as *const std::os::raw::c_char,
            _ => b"unknown error\0".as_ptr() as *const std::os::raw::c_char,
        }
    }
}

use ffi::*;

pub struct VoiceSynth {
    #[cfg(target_os = "android")]
    synthesizer: NonNull<VoicevoxSynthesizer>,
    #[cfg(target_os = "android")]
    open_jtalk: NonNull<OpenJtalkRc>,
    #[cfg(target_os = "android")]
    loaded_vvms: std::collections::HashSet<std::path::PathBuf>,
    #[cfg(not(target_os = "android"))]
    _phantom: std::marker::PhantomData<()>,
}

impl VoiceSynth {
    pub fn new(ort_lib_filename: &str, dict_dir: &Path) -> Result<Self, String> {
        #[cfg(target_os = "android")]
        {
            let ort_filename = CString::new(ort_lib_filename).map_err(|_| "invalid ort filename")?;
            let dict_dir_str = dict_dir.to_str().ok_or("invalid dict dir path")?;
            let dict_dir_c = CString::new(dict_dir_str).map_err(|_| "invalid dict dir path")?;

            let mut onnxruntime_ptr: *const VoicevoxOnnxruntime = std::ptr::null();
            let load_options = VoicevoxLoadOnnxruntimeOptions {
                filename: ort_filename.as_ptr(),
            };
            let result = unsafe {
                voicevox_onnxruntime_load_once(load_options, &mut onnxruntime_ptr)
            };
            if result != 0 {
                return Err(ffi_error_to_string(result));
            }
            let onnxruntime = NonNull::new(onnxruntime_ptr as *mut VoicevoxOnnxruntime)
                .ok_or("onnxruntime load returned null")?;

            let mut open_jtalk_ptr: *mut OpenJtalkRc = std::ptr::null_mut();
            let result = unsafe {
                voicevox_open_jtalk_rc_new(dict_dir_c.as_ptr(), &mut open_jtalk_ptr)
            };
            if result != 0 {
                return Err(ffi_error_to_string(result));
            }
            let open_jtalk = NonNull::new(open_jtalk_ptr).ok_or("open_jtalk_new returned null")?;

            let init_options = unsafe { voicevox_make_default_initialize_options() };
            let mut init_options = init_options;
            init_options.acceleration_mode = ACCELERATION_MODE_CPU;

            let mut synthesizer_ptr: *mut VoicevoxSynthesizer = std::ptr::null_mut();
            let result = unsafe {
                voicevox_synthesizer_new(
                    onnxruntime.as_ptr(),
                    open_jtalk.as_ptr(),
                    init_options,
                    &mut synthesizer_ptr,
                )
            };
            if result != 0 {
                unsafe { voicevox_open_jtalk_rc_delete(open_jtalk_ptr) };
                return Err(ffi_error_to_string(result));
            }
            let synthesizer = NonNull::new(synthesizer_ptr).ok_or("synthesizer_new returned null")?;

            Ok(VoiceSynth {
                synthesizer,
                open_jtalk,
                loaded_vvms: std::collections::HashSet::new(),
            })
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = ort_lib_filename;
            let _ = dict_dir;
            Err("voice synthesis not available on this platform".to_string())
        }
    }

    pub fn load_vvm(&mut self, vvm_path: &Path) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            if self.loaded_vvms.contains(vvm_path) {
                return Ok(());
            }

            let path_str = vvm_path.to_str().ok_or("invalid vvm path")?;
            let path_c = CString::new(path_str).map_err(|_| "invalid vvm path")?;

            let mut voice_model_file_ptr: *mut VoicevoxVoiceModelFile = std::ptr::null_mut();
            let result = unsafe {
                voicevox_voice_model_file_open(path_c.as_ptr(), &mut voice_model_file_ptr)
            };
            if result != 0 {
                return Err(ffi_error_to_string(result));
            }
            let voice_model_file = NonNull::new(voice_model_file_ptr)
                .ok_or("voice_model_file_open returned null")?;

            let load_options = unsafe { voicevox_make_default_load_voice_model_options() };
            let result = unsafe {
                voicevox_synthesizer_load_voice_model(
                    self.synthesizer.as_ptr(),
                    voice_model_file.as_ptr(),
                    load_options,
                )
            };

            unsafe { voicevox_voice_model_file_delete(voice_model_file_ptr) };

            if result != 0 {
                return Err(ffi_error_to_string(result));
            }

            self.loaded_vvms.insert(vvm_path.to_path_buf());
            Ok(())
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = vvm_path;
            Err("voice synthesis not available on this platform".to_string())
        }
    }

    pub fn tts(&self, text: &str, style_id: u32) -> Result<Vec<u8>, String> {
        #[cfg(target_os = "android")]
        {
            let text_c = CString::new(text).map_err(|_| "invalid text")?;
            let tts_options = unsafe { voicevox_make_default_tts_options() };

            let mut out_len: usize = 0;
            let mut out_wav: *mut u8 = std::ptr::null_mut();

            let result = unsafe {
                voicevox_synthesizer_tts(
                    self.synthesizer.as_ptr(),
                    text_c.as_ptr(),
                    style_id,
                    tts_options,
                    &mut out_len,
                    &mut out_wav,
                )
            };

            if result != 0 {
                return Err(ffi_error_to_string(result));
            }

            if out_wav.is_null() || out_len == 0 {
                return Err("tts returned empty wav".to_string());
            }

            let wav_data = unsafe { std::slice::from_raw_parts(out_wav, out_len).to_vec() };
            unsafe { voicevox_wav_free(out_wav) };
            Ok(wav_data)
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = text;
            let _ = style_id;
            Err("voice synthesis not available on this platform".to_string())
        }
    }

    pub fn synthesize_dialogue(
        &mut self,
        vvm_path: &Path,
        lines: &[(u32, String)],
    ) -> Result<Vec<u8>, String> {
        self.load_vvm(vvm_path)?;

        let mut wavs = Vec::with_capacity(lines.len());
        for (style_id, text) in lines {
            let wav = self.tts(text, *style_id)?;
            wavs.push(wav);
        }

        concat_wavs(&wavs, 350)
    }
}

impl Drop for VoiceSynth {
    fn drop(&mut self) {
        #[cfg(target_os = "android")]
        {
            unsafe {
                voicevox_synthesizer_delete(self.synthesizer.as_ptr());
                voicevox_open_jtalk_rc_delete(self.open_jtalk.as_ptr());
            }
        }
    }
}

fn ffi_error_to_string(code: VoicevoxResultCode) -> String {
    let msg_ptr = unsafe { voicevox_error_result_to_message(code) };
    if msg_ptr.is_null() {
        format!("voicevox error code {}", code)
    } else {
        unsafe { CStr::from_ptr(msg_ptr).to_string_lossy().into_owned() }
    }
}

#[derive(Debug, Clone, Copy)]
struct WavHeader {
    sample_rate: u32,
    num_channels: u16,
    bits_per_sample: u16,
    data_size: u32,
}

fn parse_wav_header(data: &[u8]) -> Result<(WavHeader, usize), String> {
    if data.len() < 44 {
        return Err("wav too short".to_string());
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a valid RIFF/WAVE".to_string());
    }

    let mut offset = 12;
    let mut fmt_found = false;
    let mut data_offset = 0;
    let mut header = WavHeader {
        sample_rate: 0,
        num_channels: 0,
        bits_per_sample: 0,
        data_size: 0,
    };

    while offset + 8 <= data.len() {
        let chunk_id = &data[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;

        if offset + 8 + chunk_size > data.len() {
            return Err("chunk exceeds file size".to_string());
        }

        if chunk_id == b"fmt " {
            if chunk_size < 16 {
                return Err("fmt chunk too small".to_string());
            }
            let fmt_data = &data[offset + 8..offset + 8 + chunk_size];
            let audio_format = u16::from_le_bytes([fmt_data[0], fmt_data[1]]);
            if audio_format != 1 {
                return Err("only PCM format supported".to_string());
            }
            header.num_channels = u16::from_le_bytes([fmt_data[2], fmt_data[3]]);
            header.sample_rate = u32::from_le_bytes([
                fmt_data[4], fmt_data[5], fmt_data[6], fmt_data[7],
            ]);
            header.bits_per_sample = u16::from_le_bytes([fmt_data[14], fmt_data[15]]);
            fmt_found = true;
        } else if chunk_id == b"data" {
            data_offset = offset + 8;
            header.data_size = chunk_size as u32;
            break;
        }

        offset += 8 + chunk_size;
        if chunk_size % 2 == 1 {
            offset += 1;
        }
    }

    if !fmt_found {
        return Err("fmt chunk not found".to_string());
    }
    if data_offset == 0 {
        return Err("data chunk not found".to_string());
    }

    Ok((header, data_offset))
}

pub fn concat_wavs(wavs: &[Vec<u8>], gap_ms: u32) -> Result<Vec<u8>, String> {
    if wavs.is_empty() {
        return Err("no wavs to concat".to_string());
    }

    let (first_header, _) = parse_wav_header(&wavs[0])?;
    let sample_rate = first_header.sample_rate;
    let num_channels = first_header.num_channels;
    let bits_per_sample = first_header.bits_per_sample;

    if sample_rate == 0 || num_channels == 0 || bits_per_sample == 0 {
        return Err("invalid wav format".to_string());
    }

    let mut all_pcm = Vec::new();
    let bytes_per_sample = (bits_per_sample / 8) as usize;
    let gap_samples = (sample_rate as u64 * gap_ms as u64 / 1000) as usize;
    let gap_bytes = gap_samples * num_channels as usize * bytes_per_sample;

    for (i, wav) in wavs.iter().enumerate() {
        let (header, data_offset) = parse_wav_header(wav)?;

        if header.sample_rate != sample_rate
            || header.num_channels != num_channels
            || header.bits_per_sample != bits_per_sample
        {
            return Err(format!("wav {} format mismatch", i));
        }

        let pcm_data = &wav[data_offset..data_offset + header.data_size as usize];
        all_pcm.extend_from_slice(pcm_data);

        if i + 1 < wavs.len() && gap_bytes > 0 {
            all_pcm.extend(std::iter::repeat(0u8).take(gap_bytes));
        }
    }

    let data_size = all_pcm.len() as u32;
    let riff_size = 36 + data_size;

    let mut out = Vec::with_capacity(44 + all_pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&num_channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * num_channels as u32 * bytes_per_sample as u32;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = num_channels * bytes_per_sample as u16;
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend(all_pcm);

    Ok(out)
}

#[cfg(test)]
mod oracle;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_wav(duration_ms: u32) -> Vec<u8> {
        let sample_rate = 24000u32;
        let num_channels = 1u16;
        let bits_per_sample = 16u16;
        let bytes_per_sample = (bits_per_sample / 8) as usize;
        let num_samples = (sample_rate as u64 * duration_ms as u64 / 1000) as usize;
        let data_size = num_samples * num_channels as usize * bytes_per_sample;

        let mut wav = Vec::with_capacity(44 + data_size);
        wav.extend_from_slice(b"RIFF");
        let riff_size = 36 + data_size as u32;
        wav.extend_from_slice(&riff_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&num_channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        let byte_rate = sample_rate * num_channels as u32 * bytes_per_sample as u32;
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        let block_align = num_channels * bytes_per_sample as u16;
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data_size as u32).to_le_bytes());
        wav.extend(std::iter::repeat(0u8).take(data_size));

        wav
    }

    #[test]
    fn test_concat_wavs_basic() {
        let wav1 = make_test_wav(100);
        let wav2 = make_test_wav(200);
        let result = concat_wavs(&[wav1, wav2], 350).expect("concat failed");

        let (header, _) = parse_wav_header(&result).expect("parse failed");
        assert_eq!(header.sample_rate, 24000);
        assert_eq!(header.num_channels, 1);
        assert_eq!(header.bits_per_sample, 16);

        let expected_samples = (24000 * 100 / 1000) + (24000 * 350 / 1000) + (24000 * 200 / 1000);
        let expected_data_bytes = expected_samples * 2;
        assert_eq!(header.data_size as usize, expected_data_bytes);
        assert_eq!(result.len(), 44 + expected_data_bytes);
    }
}use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy)]
pub struct VoiceEntry {
    pub key: &'static str,
    pub name: &'static str,
    pub credit: &'static str,
    pub terms_url: &'static str,
    pub vvm_file: &'static str,
    pub style_id: u32,
    pub size_mb: u32,
}

pub const VVM_BASE_URL: &str = "https://github.com/VOICEVOX/voicevox_vvm/releases/download/0.17.1";

pub const VOICE_CATALOG: &[VoiceEntry] = &[
    VoiceEntry {
        key: "zundamon",
        name: "ずんだもん",
        credit: "VOICEVOX:ずんだもん",
        terms_url: "https://zunko.jp/con_ongen_kiyaku.html",
        vvm_file: "0.vvm",
        style_id: 3,
        size_mb: 59,
    },
    VoiceEntry {
        key: "metan",
        name: "四国めたん",
        credit: "VOICEVOX:四国めたん",
        terms_url: "https://zunko.jp/con_ongen_kiyaku.html",
        vvm_file: "0.vvm",
        style_id: 2,
        size_mb: 59,
    },
    VoiceEntry {
        key: "zunko",
        name: "東北ずん子",
        credit: "VOICEVOX:東北ずん子",
        terms_url: "https://zunko.jp/con_ongen_kiyaku.html",
        vvm_file: "21.vvm",
        style_id: 107,
        size_mb: 63,
    },
    VoiceEntry {
        key: "kiritan",
        name: "東北きりたん",
        credit: "VOICEVOX:東北きりたん",
        terms_url: "https://zunko.jp/con_ongen_kiyaku.html",
        vvm_file: "21.vvm",
        style_id: 108,
        size_mb: 63,
    },
];

pub fn voice_by_key(key: &str) -> Option<&'static VoiceEntry> {
    VOICE_CATALOG.iter().find(|v| v.key == key)
}

impl VoiceEntry {
    pub fn vvm_url(&self) -> String {
        format!("{}/{}", VVM_BASE_URL, self.vvm_file)
    }
}

pub fn load_consents(data_dir: &Path) -> Vec<String> {
    let path = data_dir.join("voice_consents.json");
    if !path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<String>>(&content) {
        Ok(v) => v,
        Err(_) => Vec::new(),
    }
}

pub fn has_consent(data_dir: &Path, key: &str) -> bool {
    load_consents(data_dir).iter().any(|k| k == key)
}

pub fn record_consent(data_dir: &Path, key: &str) -> Result<(), String> {
    let mut consents = load_consents(data_dir);
    if consents.iter().any(|k| k == key) {
        return Ok(());
    }
    consents.push(key.to_string());
    let path = data_dir.join("voice_consents.json");
    let json = serde_json::to_string_pretty(&consents).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

pub fn build_script_prompt(nb_title: &str, source_text: &str, focus: Option<&str>) -> String {
    let focus_line = focus.map_or(String::new(), |f| format!("特に{}を中心に", f));
    format!(
        r#"あなたは音声番組の台本作家です。二人のホスト A（説明役）と B（聞き役）の対話台本を書いてください。

形式：
- 1行ごとに「A: 」または「B: 」で始め、交互に話す
- 全体で 1200〜2000 字程度
- 資料の内容だけを使い、資料に無いことは言わない
- 読みにくい漢字の語は自然な範囲でひらがなにする
- ト書き・見出し・引用符の装飾は付けない
{}
最後に資料本文を渡します。

資料タイトル: {}
資料本文:
{}"#,
        if focus_line.is_empty() { String::new() } else { format!("{}\n", focus_line) },
        nb_title,
        source_text
    )
}

pub fn parse_script(text: &str) -> Result<Vec<(u8, String)>, String> {
    let mut result = Vec::new();
    let mut has_a = false;
    let mut has_b = false;

    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = if let Some(c) = trimmed.strip_prefix("A:").or_else(|| trimmed.strip_prefix("A：")) {
            Some((0u8, c))
        } else if let Some(c) = trimmed.strip_prefix("B:").or_else(|| trimmed.strip_prefix("B：")) {
            Some((1u8, c))
        } else {
            None
        };
        if let Some((who, content)) = parsed {
            let content = content.trim();
            if content.is_empty() {
                continue;
            }
            if who == 0 { has_a = true; } else { has_b = true; }
            result.push((who, content.to_string()));
        }
    }

    if !has_a || !has_b {
        return Err("A と B の両方が最低1行ずつ必要です".to_string());
    }
    Ok(result)
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AudioSummary {
    pub id: u64,
    pub nb: u64,
    pub title: String,
    pub wav_file: String,
    pub voice_a: String,
    pub voice_b: String,
    pub created_at: u64,
    pub duration_sec: f64,
}

pub fn load_summaries(data_dir: &Path) -> Vec<AudioSummary> {
    let path = data_dir.join("audio_summaries.jsonl");
    if !path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            serde_json::from_str::<AudioSummary>(line).ok()
        })
        .collect()
}

pub fn append_summary(data_dir: &Path, rec: &AudioSummary) -> Result<(), String> {
    let path = data_dir.join("audio_summaries.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    let json = serde_json::to_string(rec).map_err(|e| e.to_string())?;
    writeln!(file, "{}", json).map_err(|e| e.to_string())
}

pub fn remove_summary(data_dir: &Path, id: u64) -> Result<Option<AudioSummary>, String> {
    let path = data_dir.join("audio_summaries.jsonl");
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut removed = None;
    let mut new_lines = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<AudioSummary>(line) {
            if rec.id == id {
                removed = Some(rec);
                continue;
            }
        }
        new_lines.push(line.to_string());
    }
    let body = if new_lines.is_empty() { String::new() } else { new_lines.join("\n") + "\n" };
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(removed)
}

pub fn wav_duration_sec(wav: &[u8]) -> Option<f64> {
    let (header, _) = parse_wav_header(wav).ok()?;
    let bytes_per_sample = (header.bits_per_sample / 8) as u32;
    let bytes_per_sec = header.sample_rate * (header.num_channels as u32) * bytes_per_sample;
    if bytes_per_sec == 0 {
        return None;
    }
    Some(header.data_size as f64 / bytes_per_sec as f64)
}
pub fn load_voice_pair(data_dir: &Path) -> (String, String) {
    let default = ("zundamon".to_string(), "metan".to_string());
    let path = data_dir.join("voice_pair.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return default;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else {
        return default;
    };
    let pick = |k: &str, fallback: String| -> String {
        match v.get(k).and_then(|x| x.as_str()) {
            Some(s) if voice_by_key(s).is_some() => s.to_string(),
            _ => fallback,
        }
    };
    (pick("a", default.0), pick("b", default.1))
}

pub fn save_voice_pair(data_dir: &Path, a: &str, b: &str) -> Result<(), String> {
    if voice_by_key(a).is_none() {
        return Err(format!("unknown voice: {a}"));
    }
    if voice_by_key(b).is_none() {
        return Err(format!("unknown voice: {b}"));
    }
    if a == b {
        return Err("the two hosts must be different voices".to_string());
    }
    let body = serde_json::json!({ "a": a, "b": b }).to_string();
    std::fs::write(data_dir.join("voice_pair.json"), body).map_err(|e| e.to_string())
}
