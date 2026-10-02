// Empêche l'ouverture d'une console sous Windows en version finale.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lumen_lib::run()
}
