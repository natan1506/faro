// Sem janela de console no Windows (o Vigia é para macOS, mas o build não quebra).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    vigia_app_lib::run()
}
