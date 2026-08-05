use std::{env, process::Command, thread, time::Duration};

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("desktop-root") => {
            let child = args.next().expect("desktop-root requires child path");
            Command::new(child)
                .arg("desktop-child")
                .spawn()
                .expect("launch desktop child");
        }
        Some("desktop-child") | Some("cli") => {}
        _ => {}
    }
    thread::sleep(Duration::from_secs(120));
}
