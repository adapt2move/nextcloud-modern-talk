//! Private running-image fixture only; no network, real package manager or checkout mutation.
use std::{
    path::PathBuf,
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 || args[0] != "synthetic-update-fixture" {
        eprintln!("This executable requires the private install_probe.py fixture.");
        return ExitCode::FAILURE;
    }
    let original = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return ExitCode::FAILURE,
    };
    let archive = PathBuf::from(&args[1]);
    let Some(digest) = args[2].to_str() else {
        return ExitCode::FAILURE;
    };
    let installed = match talk_adapters::updates::fixture::install(&archive, digest) {
        Ok(path) => path,
        Err(error) => {
            println!("REFUSED {error:?}");
            return ExitCode::FAILURE;
        }
    };
    if installed != original {
        return ExitCode::FAILURE;
    }
    #[cfg(target_os = "linux")]
    {
        let relocated = std::env::current_exe().ok();
        if relocated.as_deref() == Some(original.as_path())
            || relocated.as_ref().is_some_and(|p| p.exists())
        {
            eprintln!("Expected the old running image to have moved to the removed stash.");
            return ExitCode::FAILURE;
        }
        println!("OLD_RUNNING_IMAGE_RELOCATED");
    }
    // Launch the captured original installed path, never the late current_exe path.
    let result = Command::new(&installed).arg("synthetic-restart").output();
    match result {
        Ok(output) if output.status.success() && output.stdout == b"NMT UPDATE FIXTURE NEW\n" => {
            println!("INSTALLED_AND_RELAUNCHED_EXACT_STAGED_FIXTURE");
            ExitCode::SUCCESS
        }
        _ => ExitCode::FAILURE,
    }
}
