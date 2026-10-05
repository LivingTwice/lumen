//! Windows : Vulkan, par lequel llama.cpp et whisper.cpp calculent sur la carte
//! graphique, chargé seulement s'il est là.
//!
//! Leur moteur Vulkan (ggml) trouve presque toutes ses fonctions à l'exécution,
//! par `vkGetInstanceProcAddr` ; il n'en appelle que quatre directement. Lumen les
//! fournit ici, à la place de la bibliothèque du SDK de Vulkan (une bibliothèque
//! vide en tient lieu à la compilation, voir scripts/build-windows.mjs) : elles
//! ouvrent au premier appel vulkan-1.dll, que le pilote de la carte graphique
//! installe dans le dossier système de Windows. Sans elle (machine virtuelle, PC
//! sans pilote graphique), Vulkan répond « indisponible », ggml l'écarte et tout
//! se calcule sur le processeur. Liée directement, une vulkan-1.dll manquante
//! empêcherait Lumen de démarrer.
//!
//! Si une nouvelle version de llama.cpp ou de whisper.cpp appelle directement une
//! autre fonction de Vulkan, la compilation pour Windows échoue sur son nom : il
//! suffit de l'ajouter ici sur le même modèle.
//!
//! Partagé avec lumen-whisper (`#[path]`) : rien d'autre que la bibliothèque
//! standard et kernel32.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

type Proc = unsafe extern "system" fn();
type Handle = *mut c_void;
type GetProc = unsafe extern "system" fn(Handle, *const c_char) -> Option<Proc>;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(module: Handle, name: *const c_char) -> Option<Proc>;
}

/// vulkan-1.dll, cherchée seulement dans le dossier système (jamais une copie posée ailleurs)
fn library() -> Handle {
    static LIB: OnceLock<usize> = OnceLock::new();
    *LIB.get_or_init(|| {
        const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
        let name: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
        unsafe { LoadLibraryExW(name.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) as usize }
    }) as Handle
}

/// Une fonction de vulkan-1.dll (`name` finit par un zéro), cherchée une seule fois.
fn symbol(slot: &OnceLock<Option<Proc>>, name: &str) -> Option<Proc> {
    *slot.get_or_init(|| {
        let lib = library();
        if lib.is_null() {
            return None;
        }
        unsafe { GetProcAddress(lib, name.as_ptr().cast()) }
    })
}

/// Vulkan absent : chaque fonction demandée répond VK_ERROR_INITIALIZATION_FAILED.
/// Windows x64 n'a qu'une convention d'appel : les arguments passés à cette
/// fonction, quels qu'ils soient, sont simplement ignorés.
unsafe extern "system" fn unavailable() -> i32 {
    -3
}

/// Vulkan est là : vulkan-1.dll s'ouvre (la question se pose une seule fois).
#[allow(dead_code)] // lumen-whisper n'en a pas besoin
pub fn present() -> bool {
    !library().is_null()
}

#[no_mangle]
pub unsafe extern "system" fn vkGetInstanceProcAddr(instance: Handle, name: *const c_char) -> Option<Proc> {
    static F: OnceLock<Option<Proc>> = OnceLock::new();
    match symbol(&F, "vkGetInstanceProcAddr\0") {
        Some(f) => std::mem::transmute::<Proc, GetProc>(f)(instance, name),
        // ggml demande d'abord la version de Vulkan : « indisponible », il renonce à la carte graphique
        None => Some(std::mem::transmute::<unsafe extern "system" fn() -> i32, Proc>(unavailable)),
    }
}

// Les trois suivantes ne servent qu'une fois Vulkan démarré, donc vulkan-1.dll ouverte.

#[no_mangle]
pub unsafe extern "system" fn vkGetDeviceProcAddr(device: Handle, name: *const c_char) -> Option<Proc> {
    static F: OnceLock<Option<Proc>> = OnceLock::new();
    let f = symbol(&F, "vkGetDeviceProcAddr\0")?;
    std::mem::transmute::<Proc, GetProc>(f)(device, name)
}

#[no_mangle]
pub unsafe extern "system" fn vkGetPhysicalDeviceFeatures2(device: Handle, features: *mut c_void) {
    static F: OnceLock<Option<Proc>> = OnceLock::new();
    if let Some(f) = symbol(&F, "vkGetPhysicalDeviceFeatures2\0") {
        std::mem::transmute::<Proc, unsafe extern "system" fn(Handle, *mut c_void)>(f)(device, features)
    }
}

#[no_mangle]
pub unsafe extern "system" fn vkCmdCopyBuffer(commands: Handle, src: u64, dst: u64, count: u32, regions: *const c_void) {
    static F: OnceLock<Option<Proc>> = OnceLock::new();
    if let Some(f) = symbol(&F, "vkCmdCopyBuffer\0") {
        std::mem::transmute::<Proc, unsafe extern "system" fn(Handle, u64, u64, u32, *const c_void)>(f)(commands, src, dst, count, regions)
    }
}

#[cfg(test)]
mod tests {
    /// Sur un vrai Windows : la première question de ggml reçoit une réponse, avec ou sans Vulkan.
    #[test]
    fn answers_with_or_without_vulkan() {
        let f = unsafe { super::vkGetInstanceProcAddr(std::ptr::null_mut(), c"vkEnumerateInstanceVersion".as_ptr()) }.expect("fonction");
        let f = unsafe { std::mem::transmute::<super::Proc, unsafe extern "system" fn(*mut u32) -> i32>(f) };
        let mut version = 0u32;
        let r = unsafe { f(&mut version) };
        if super::present() {
            assert_eq!(r, 0);
            assert!(version >= 1 << 22, "Vulkan 1.0 au moins");
        } else {
            assert_eq!(r, -3, "VK_ERROR_INITIALIZATION_FAILED");
        }
    }
}
