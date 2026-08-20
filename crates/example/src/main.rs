mod scenes;

use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use scenes::Step;

struct TimelineEntry {
    time: String,
    title: &'static str,
    records: &'static str,
}

fn usage() {
    println!("Usage: example start [--at <path>]\n       example --help");
}

fn main() {
    match parse_args() {
        Ok(Some(path)) => run(path),
        Ok(None) => usage(),
        Err(message) => {
            eprintln!("{message}");
            usage();
            std::process::exit(2);
        }
    }
}

fn parse_args() -> Result<Option<Option<PathBuf>>, String> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    if args.first().is_none_or(|arg| arg != "start") {
        return Err("expected `start` or `--help`".to_owned());
    }
    match args.len() {
        1 => Ok(Some(None)),
        3 if args[1] == "--at" => Ok(Some(Some(PathBuf::from(&args[2])))),
        _ => Err("expected `example start [--at <path>]`".to_owned()),
    }
}

fn run(requested_path: Option<PathBuf>) {
    ensure_lane_available();
    let sandbox = match create_sandbox(requested_path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("could not create tour sandbox: {error}");
            std::process::exit(1);
        }
    };

    println!("\nSandbox created: {}\n", sandbox.display());
    print_wrapped(scenes::OPENING, 0, "");

    let mut played = vec![false; scenes::SCENES.len()];
    let mut timeline = Vec::new();
    loop {
        print_menu(&sandbox, &played);
        let Some(choice) = read_line("\nChoose an option: ") else {
            println!();
            break;
        };
        let choice = choice.trim();
        match choice {
            "t" => print_timeline(&timeline),
            "d" => print_tree(&sandbox),
            "n" => {
                if let Some(index) = recommended(&played) {
                    play_scene(index, &sandbox, &mut played, &mut timeline);
                } else {
                    println!("\nEvery scene has been played.");
                }
            }
            "q" => {
                println!();
                print_wrapped(scenes::CLOSING, 0, "");
                print_timeline(&timeline);
                println!("\nSandbox: {}", sandbox.display());
                println!("Delete it when you are done: rm -rf {}", sandbox.display());
                break;
            }
            _ => {
                if let Some(index) = scenes::SCENES.iter().position(|scene| scene.key == choice) {
                    if recommended(&played).is_some_and(|next| next != index) {
                        println!(
                            "\nWarning: this scene normally follows earlier scenes; it may depend on their changes."
                        );
                    }
                    play_scene(index, &sandbox, &mut played, &mut timeline);
                } else {
                    println!("\nUnknown option `{choice}`.");
                }
            }
        }
    }
}

fn ensure_lane_available() {
    match Command::new("lane").arg("--version").output() {
        Ok(output) if output.status.success() => {}
        Ok(_) | Err(_) => {
            eprintln!(
                "`lane` is not available on PATH. Install it with `cargo install --path crates/lane` and try again."
            );
            std::process::exit(1);
        }
    }
}

fn create_sandbox(requested_path: Option<PathBuf>) -> io::Result<PathBuf> {
    let path = match requested_path {
        Some(path) => absolute_path(path)?,
        None => next_sandbox_path()?,
    };
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", path.display()),
        ));
    }
    fs::create_dir_all(&path)?;
    fs::create_dir(path.join("src"))?;
    fs::write(
        path.join("src/auth.rs"),
        "pub fn verify(token: &str) -> bool {\n    parse(token).is_valid()\n}\n",
    )?;
    fs::write(path.join(".gitignore"), "target/\n")?;

    run_checked(&path, "git", &["init", "-b", "main"])?;
    run_checked(&path, "git", &["config", "user.name", "Lane Tour"])?;
    run_checked(
        &path,
        "git",
        &["config", "user.email", "tour@example.invalid"],
    )?;
    run_checked(&path, "git", &["config", "init.defaultBranch", "main"])?;
    run_checked(&path, "git", &["add", "-A"])?;
    run_checked(&path, "git", &["commit", "-q", "-m", "start sandbox"])?;
    run_checked(&path, "lane", &["init"])?;
    run_checked(&path, "git", &["add", "-A"])?;
    run_checked(&path, "git", &["commit", "-q", "-m", "initialize lane"])?;
    Ok(path)
}

fn absolute_path(path: PathBuf) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn next_sandbox_path() -> io::Result<PathBuf> {
    let parent = env::current_dir()?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("current directory has no parent"))?;
    for number in 1_u64.. {
        let candidate = parent.join(format!("lane-example-{number}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn run_checked(cwd: &Path, program: &str, args: &[&str]) -> io::Result<()> {
    let output = Command::new(program).args(args).current_dir(cwd).output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "`{program} {}` failed: {}{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )))
}

fn print_menu(sandbox: &Path, played: &[bool]) {
    println!("\n--- Lane tour ---");
    println!("Sandbox: {}", sandbox.display());
    for (index, scene) in scenes::SCENES.iter().enumerate() {
        let marker = if played[index] { "played" } else { "      " };
        println!("  {}  {} — {}", scene.key, marker, scene.title);
    }
    if let Some(index) = recommended(played) {
        println!("  n         — play next ({})", scenes::SCENES[index].key);
    } else {
        println!("  n         — all scenes played");
    }
    println!("  t         — timeline");
    println!("  d         — sandbox tree and history");
    println!("  q         — finish and keep the sandbox path");
}

fn recommended(played: &[bool]) -> Option<usize> {
    played.iter().position(|was_played| !was_played)
}

fn play_scene(
    index: usize,
    sandbox: &Path,
    played: &mut [bool],
    timeline: &mut Vec<TimelineEntry>,
) {
    let scene = &scenes::SCENES[index];
    println!("\n=== {}: {} ===", scene.key, scene.title);
    print_wrapped(scene.why, 2, "Why: ");
    for step in scene.steps {
        match step {
            Step::Say(text) => print_wrapped(text, 2, "• "),
            Step::Do(command) => run_scene_command(sandbox, command),
            Step::In(directory, command) => run_scene_command(&sandbox.join(directory), command),
            Step::Look(text) => {
                print_wrapped(text, 2, "Look: ");
                println!("  [enter] to continue");
                let _ = read_line("");
            }
        }
    }
    played[index] = true;
    timeline.push(TimelineEntry {
        time: wall_clock(),
        title: scene.title,
        records: scene.records,
    });
}

fn run_scene_command(cwd: &Path, command: &str) {
    println!("  $ {command}");
    match Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()
    {
        Ok(output) => print_output(&output),
        Err(error) => println!("    could not run command: {error}"),
    }
}

fn print_output(output: &Output) {
    print_indented(&String::from_utf8_lossy(&output.stdout));
    print_indented(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        match output.status.code() {
            Some(code) => println!("    exit: {code}"),
            None => println!("    command terminated by signal"),
        }
    }
}

fn print_tree(sandbox: &Path) {
    println!("\n--- Sandbox tree ---");
    for command in ["find .lane -print 2>/dev/null", "git log --oneline"] {
        run_scene_command(sandbox, command);
    }
}

fn print_timeline(timeline: &[TimelineEntry]) {
    println!("\n--- Timeline ---");
    if timeline.is_empty() {
        println!("  Nothing recorded yet.");
    }
    for entry in timeline {
        println!("  {} — {}: {}", entry.time, entry.title, entry.records);
    }
}

fn wall_clock() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let seconds = duration.as_secs() % 86_400;
            format!(
                "{:02}:{:02}:{:02} UTC",
                seconds / 3_600,
                seconds / 60 % 60,
                seconds % 60
            )
        }
        Err(_) => "before Unix epoch".to_owned(),
    }
}

fn read_line(prompt: &str) -> Option<String> {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut input = String::new();
    (io::stdin().read_line(&mut input).ok()? != 0).then_some(input)
}

fn print_wrapped(text: &str, indent: usize, first_prefix: &str) {
    const WIDTH: usize = 78;
    let prefix_width = first_prefix.chars().count();
    for (index, paragraph) in text.split("\n\n").enumerate() {
        if index > 0 {
            println!();
        }
        let mut padding = " ".repeat(indent);
        let mut line = String::new();
        let mut prefix = first_prefix;
        for word in paragraph.split_whitespace() {
            let width = padding.len()
                + prefix.chars().count()
                + line.chars().count()
                + usize::from(!line.is_empty());
            if width + word.chars().count() > WIDTH && !line.is_empty() {
                println!("{padding}{prefix}{line}");
                line.clear();
                padding = " ".repeat(indent + prefix_width);
                prefix = "";
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        println!("{padding}{prefix}{line}");
    }
}

fn print_indented(text: &str) {
    for line in text.lines() {
        println!("    {line}");
    }
}

#[cfg(test)]
mod tests {
    use super::{print_wrapped, recommended};

    #[test]
    fn chooses_first_unplayed_scene() {
        assert_eq!(recommended(&[true, false, false]), Some(1));
        assert_eq!(recommended(&[true, true]), None);
    }

    #[test]
    fn wraps_prose() {
        print_wrapped("short text", 2, "• ");
    }
}
