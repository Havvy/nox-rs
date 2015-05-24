extern crate argparse;

use argparse::{ArgumentParser, Store};

fn main() {
    let mut query = "".to_string();

    {
        let mut cli = ArgumentParser::new();
        cli.set_description("Search and install Nix packages.");
        cli.refer(&mut query)
            .add_argument("query", Store, "Package to search for.")
            .required();
        cli.parse_args_or_exit();
    }

    let query: &str = &query;

    println!("Query: {}", query);
}
