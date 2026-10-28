//! Scriptable counterpart to the launcher repository flow.
pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let result = execute(args);
    result.map(|()| 0).map_err(|e| e.to_string().into())
}
fn execute(args: &[String]) -> fanplay_provision::Result<()> {
    use fanplay_provision as p;
    let root = p::root(None);
    match args.first().map(String::as_str) {
        Some("list") if args.len() >= 2 => {
            for source in &args[1..] {
                let repo = p::load_repository(source)?;
                for game in &repo.games { println!("{}/{}\t{}\t{:?}\t{}", repo.id, game.id, game.title, game.platform, game.version); }
            }
        }
        Some("install") if args.len() == 3 => {
            let repo = p::load_repository(&args[1])?;
            let game = repo.games.iter().find(|g| g.id == args[2]).ok_or("Game id not found")?;
            println!("{}", p::install(&root, &repo, game)?.display());
        }
        Some("import-base") if args.len() == 2 => println!("{}", p::import_base(&root, std::path::Path::new(&args[1]))?),
        _ => return Err("Usage: fanplay software-provision list REPOSITORY... | install REPOSITORY GAME_ID | import-base ROM".into()),
    }
    Ok(())
}
