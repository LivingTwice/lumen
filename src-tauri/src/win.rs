//! Ce que la version Windows fait autrement que macOS : compresser le son en
//! AAC (Media Foundation, à la place d'`afconvert`), faire lire à l'outil de la
//! voix son texte en UTF-8, reconnaître le PC (à la place d'`ioreg`), compter
//! les cœurs du processeur.

use std::path::Path;

use anyhow::{anyhow, Result};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Compresse un son PCM 16 bits mono en AAC dans un fichier .m4a, avec
/// l'encodeur de Windows. Il n'accepte que 44,1 et 48 kHz : une autre fréquence
/// (24 kHz pour les voix de Gemini) est convertie avant.
pub fn aac_m4a(rate: u32, samples: &[i16], out: &Path) -> Result<()> {
    let converted;
    let (rate, samples) = if rate == 44_100 || rate == 48_000 {
        (rate, samples)
    } else {
        converted = crate::voice::resample(samples, rate, 48_000);
        (48_000, converted.as_slice())
    };
    let _ = std::fs::remove_file(out);
    // Media Foundation absent (Windows « N » sans le Media Feature Pack) : le son reste en
    // WAV. Ses bibliothèques ne sont chargées qu'ici (build.rs) : on vérifie d'abord.
    if !media_foundation() {
        return Err(anyhow!("Media Foundation absent"));
    }
    let res = unsafe {
        // COM sur ce fil de travail, puis Media Foundation, refermés dans l'ordre inverse
        let com = CoInitializeEx(None, COINIT_MULTITHREADED);
        let res = MFStartup(MF_VERSION, MFSTARTUP_LITE).and_then(|_| {
            let r = encode(rate, samples, out);
            let _ = MFShutdown();
            r
        });
        if com.is_ok() {
            CoUninitialize();
        }
        res
    };
    res.map_err(|e| anyhow!("Media Foundation : {e}"))?;
    if !out.is_file() {
        return Err(anyhow!("Media Foundation : fichier absent"));
    }
    Ok(())
}

/// Les bibliothèques de Media Foundation sont-elles là ?
fn media_foundation() -> bool {
    use windows::Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    ["mfplat.dll", "mfreadwrite.dll"].iter().all(|dll| {
        let name = HSTRING::from(*dll);
        unsafe { LoadLibraryExW(PCWSTR(name.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.is_ok()
    })
}

/// Écrit le fichier (Media Foundation déjà démarré).
fn encode(rate: u32, samples: &[i16], out: &Path) -> windows::core::Result<()> {
    unsafe {
        // conteneur MPEG-4 (.m4a), quel que soit le nom du fichier
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        let path = HSTRING::from(out.as_os_str());
        let writer = MFCreateSinkWriterFromURL(PCWSTR(path.as_ptr()), None::<&IMFByteStream>, &attrs)?;

        // sortie : AAC mono, 96 kbit/s (le plus petit débit de l'encodeur de Windows)
        let target = MFCreateMediaType()?;
        target.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        target.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        target.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        target.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        target.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)?;
        target.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 12_000)?;
        let stream = writer.AddStream(&target)?;

        // entrée : PCM 16 bits mono
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)?;
        input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 2)?;
        input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * 2)?;
        writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>)?;
        writer.BeginWriting()?;

        // une seconde à la fois ; le temps se compte en centaines de nanosecondes
        let mut done = 0i64;
        for chunk in samples.chunks(rate as usize) {
            let bytes = (chunk.len() * 2) as u32;
            let buffer = MFCreateMemoryBuffer(bytes)?;
            let mut data: *mut u8 = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(chunk.as_ptr() as *const u8, data, bytes as usize);
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(done * 10_000_000 / rate as i64)?;
            sample.SetSampleDuration(chunk.len() as i64 * 10_000_000 / rate as i64)?;
            writer.WriteSample(stream, &sample)?;
            done += chunk.len() as i64;
        }
        writer.Finalize()
    }
}

/// Manifeste qui fait lire à un programme sa ligne de commande en UTF-8.
const UTF8_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
</assembly>
"#;

/// L'outil de la voix reçoit le texte à dire sur sa ligne de commande, que
/// Windows lui passe dans la page de code du système (le russe, le japonais,
/// l'arabe y deviendraient des « ? »). Son manifeste est remplacé par un autre
/// qui lui demande l'UTF-8 (Windows 10 de mai 2019 et après).
pub fn utf8_manifest(exe: &Path) -> Result<()> {
    use windows::Win32::System::LibraryLoader::{BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW};
    // RT_MANIFEST (24), ressource n° 1 : le manifeste d'un programme
    let kind = PCWSTR(24 as *const u16);
    let id = PCWSTR(1 as *const u16);
    let path = HSTRING::from(exe.as_os_str());
    unsafe {
        let h = BeginUpdateResourceW(PCWSTR(path.as_ptr()), false)?;
        // l'ancien manifeste, en langue neutre ou en anglais
        for lang in [0u16, 1033] {
            let _ = UpdateResourceW(h, kind, id, lang, None, 0);
        }
        let res = UpdateResourceW(h, kind, id, 1033, Some(UTF8_MANIFEST.as_ptr() as *const _), UTF8_MANIFEST.len() as u32);
        EndUpdateResourceW(h, res.is_err())?;
        res?;
    }
    Ok(())
}

/// Cœurs physiques du processeur (l'hyperthreading ralentit llama.cpp et Whisper).
pub fn physical_cores() -> Option<usize> {
    use windows::Win32::System::SystemInformation::{GetLogicalProcessorInformation, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION};
    let size = std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
    let mut len = 0u32;
    unsafe {
        let _ = GetLogicalProcessorInformation(None, &mut len);
        if (len as usize) < size {
            return None;
        }
        let mut buf: Vec<SYSTEM_LOGICAL_PROCESSOR_INFORMATION> = vec![std::mem::zeroed(); len as usize / size + 1];
        GetLogicalProcessorInformation(Some(buf.as_mut_ptr()), &mut len).ok()?;
        let n = (len as usize / size).min(buf.len());
        let cores = buf[..n].iter().filter(|i| i.Relationship == RelationProcessorCore).count();
        (cores > 0).then_some(cores)
    }
}

/// Identifiant de cette installation de Windows (MachineGuid du registre),
/// stable tant que Windows n'est pas réinstallé.
pub fn machine_guid() -> Option<String> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let key = HSTRING::from("SOFTWARE\\Microsoft\\Cryptography");
    let value = HSTRING::from("MachineGuid");
    let mut buf = [0u16; 128];
    let mut size = (buf.len() * 2) as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if err.is_err() {
        return None;
    }
    let n = (size as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    let s = s.trim_matches(char::from(0)).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Lecteurs de ce PC (« D », « G »…) qui ne sont ni des lecteurs réseau (un
/// lecteur réseau débranché ferait attendre de longues secondes) ni des lecteurs
/// de disques optiques.
pub fn local_drives() -> Vec<char> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    // types de lecteur : 0 inconnu, 1 absent, 4 réseau, 5 CD ou DVD
    const SKIP: [u32; 4] = [0, 1, 4, 5];
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| (b'A' + i) as char)
        .filter(|l| {
            let root = HSTRING::from(format!("{l}:\\"));
            !SKIP.contains(&unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) })
        })
        .collect()
}

// Ces tests ne tournent que sous Windows (sur un PC, ou par .github/workflows/windows.yml).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_and_machine() {
        assert!(physical_cores().is_some_and(|n| n >= 1));
        assert!(machine_guid().is_some_and(|g| g.len() >= 32), "MachineGuid lisible");
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).chars().next().unwrap();
        assert!(local_drives().contains(&system));
    }

    #[test]
    fn lesson_audio_in_aac() {
        // Windows Server sans Media Foundation : le son resterait en WAV
        if !media_foundation() {
            return;
        }
        // deux secondes d'un la à 24 kHz (la fréquence des voix de Gemini), converties en 48 kHz
        let rate = 24_000u32;
        let s: Vec<i16> = (0..rate * 2).map(|k| ((k as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin() * 8000.0) as i16).collect();
        let out = std::env::temp_dir().join(format!("lumen-aac-{}.m4a", std::process::id()));
        aac_m4a(rate, &s, &out).expect("AAC par Media Foundation");
        let data = std::fs::read(&out).unwrap();
        assert_eq!(&data[4..8], b"ftyp", "conteneur MPEG-4");
        // 96 kbit/s : environ 24 Ko pour deux secondes
        assert!((8_000..120_000).contains(&data.len()), "taille : {}", data.len());
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn voice_tool_reads_utf8() {
        // une copie d'un programme (celui des tests) reçoit le manifeste UTF-8
        let exe = std::env::temp_dir().join(format!("lumen-manifest-{}.exe", std::process::id()));
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        utf8_manifest(&exe).expect("manifeste remplacé");
        let data = std::fs::read(&exe).unwrap();
        assert!(data.windows(22).any(|w| w == b"UTF-8</activeCodePage>"));
        let _ = std::fs::remove_file(&exe);
    }
}
