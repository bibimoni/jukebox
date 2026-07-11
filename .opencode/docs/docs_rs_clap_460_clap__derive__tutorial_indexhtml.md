# clap::_derive::_tutorial - Rust

> Source: https://docs.rs/clap/4.6.0/clap/_derive/_tutorial/index.html
> Cached: 2026-07-11T19:54:31.015Z

---

[clap](../../index.html)::[_derive](../index.html)# Module _tutorial Copy item path

[Source](../../../src/clap/_derive/_tutorial.rs.html#10-257) Available on **crate feature `unstable-doc`** only.Expand description### [§](#tutorial-for-the-derive-api)Tutorial for the Derive API

*See the side bar for the Table of Contents*

### [§](#quick-start)Quick Start

You can create an application declaratively with a `struct` and some
attributes.
First, ensure `clap` is available with the [`derive` feature flag](../../_features/index.html):

```
$ cargo add clap --features derive
```

Here is a preview of the type of application you can make:

```
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Optional name to operate on
    name: Option<String>,

    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    debug: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// does testing things
    Test {
        /// lists test values
        #[arg(short, long)]
        list: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    // You can check the value provided by positional arguments, or option arguments
    if let Some(name) = cli.name.as_deref() {
        println!("Value for name: {name}");
    }

    if let Some(config_path) = cli.config.as_deref() {
        println!("Value for config: {}", config_path.display());
    }

    // You can see how many times a particular flag or argument occurred
    // Note, only flags can have multiple occurrences
    match cli.debug {
        0 => println!("Debug mode is off"),
        1 => println!("Debug mode is kind of on"),
        2 => println!("Debug mode is on"),
        _ => println!("Don't be crazy"),
    }

    // You can check for the existence of subcommands, and if found use their
    // matches just as you would the top level cmd
    match &cli.command {
        Some(Commands::Test { list }) => {
            if *list {
                println!("Printing testing lists...");
            } else {
                println!("Not printing testing lists...");
            }
        }
        None => {}
    }

    // Continued program logic goes here...
}
```

```
$ 01_quick_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 01_quick_derive[EXE] [OPTIONS] [NAME] [COMMAND]

Commands:
  test  does testing things
  help  Print this message or the help of the given subcommand(s)

Arguments:
  [NAME]  Optional name to operate on

Options:
  -c, --config <FILE>  Sets a custom config file
  -d, --debug...       Turn debugging information on
  -h, --help           Print help
  -V, --version        Print version

```

By default, the program does nothing:

```
$ 01_quick_derive
Debug mode is off

```

But you can mix and match the various features

```
$ 01_quick_derive -dd test
Debug mode is on
Not printing testing lists...

```

See also

- [FAQ: When should I use the builder vs derive APIs?](../../_faq/index.html#when-should-i-use-the-builder-vs-derive-apis)

- The [cookbook](../../_cookbook/index.html) for more application-focused examples

### [§](#configuring-the-parser)Configuring the Parser

You use derive [`Parser`](../../trait.Parser.html) to start building a parser.

```
use clap::Parser;

#[derive(Parser)]
#[command(name = "MyApp")]
#[command(version = "1.0")]
#[command(about = "Does awesome things", long_about = None)]
struct Cli {
    #[arg(long)]
    two: String,
    #[arg(long)]
    one: String,
}

fn main() {
    let cli = Cli::parse();

    println!("two: {:?}", cli.two);
    println!("one: {:?}", cli.one);
}
```

```
$ 02_apps_derive --help
Does awesome things

Usage: 02_apps_derive[EXE] --two <TWO> --one <ONE>

Options:
      --two <TWO>  
      --one <ONE>  
  -h, --help       Print help
  -V, --version    Print version

$ 02_apps_derive --version
MyApp 1.0

```

You can use [`#[command(version, about)]` attribute defaults](../index.html#command-attributes) on the struct to fill these fields in from your `Cargo.toml` file.

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)] // Read from `Cargo.toml`
struct Cli {
    #[arg(long)]
    two: String,
    #[arg(long)]
    one: String,
}

fn main() {
    let cli = Cli::parse();

    println!("two: {:?}", cli.two);
    println!("one: {:?}", cli.one);
}
```

```
$ 02_crate_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 02_crate_derive[EXE] --two <TWO> --one <ONE>

Options:
      --two <TWO>  
      --one <ONE>  
  -h, --help       Print help
  -V, --version    Print version

$ 02_crate_derive --version
clap [..]

```

You can use `#[command]` attributes on the struct to change the application level behavior of clap.  Any [`Command`](../../struct.Command.html) builder function can be used as an attribute, like [`Command::next_line_help`](../../struct.Command.html#method.next_line_help).

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(next_line_help = true)]
struct Cli {
    #[arg(long)]
    two: String,
    #[arg(long)]
    one: String,
}

fn main() {
    let cli = Cli::parse();

    println!("two: {:?}", cli.two);
    println!("one: {:?}", cli.one);
}
```

```
$ 02_app_settings_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 02_app_settings_derive[EXE] --two <TWO> --one <ONE>

Options:
      --two <TWO>
          
      --one <ONE>
          
  -h, --help
          Print help
  -V, --version
          Print version

```

### [§](#adding-arguments)Adding Arguments

- [Positionals](#positionals)

- [Options](#options)

- [Flags](#flags)

- [Optional](#optional)

- [Defaults](#defaults)

- [Subcommands](#subcommands)

Arguments are inferred from the fields of your struct.

#### [§](#positionals)Positionals

By default, struct fields define positional arguments:

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    name: String,
}

fn main() {
    let cli = Cli::parse();

    println!("name: {:?}", cli.name);
}
```

```
$ 03_03_positional_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_03_positional_derive[EXE] <NAME>

Arguments:
  <NAME>  

Options:
  -h, --help     Print help
  -V, --version  Print version

$ 03_03_positional_derive
? 2
error: the following required arguments were not provided:
  <NAME>

Usage: 03_03_positional_derive[EXE] <NAME>

For more information, try '--help'.

$ 03_03_positional_derive bob
name: "bob"

```

Note that the [default `ArgAction` is `Set`](../index.html#arg-types).  To
accept multiple values, override the [action](../../struct.Arg.html#method.action) with [`Append`](../../enum.ArgAction.html#variant.Append) via `Vec`:

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    name: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    println!("name: {:?}", cli.name);
}
```

```
$ 03_03_positional_mult_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_03_positional_mult_derive[EXE] [NAME]...

Arguments:
  [NAME]...  

Options:
  -h, --help     Print help
  -V, --version  Print version

$ 03_03_positional_mult_derive
name: []

$ 03_03_positional_mult_derive bob
name: ["bob"]

$ 03_03_positional_mult_derive bob john
name: ["bob", "john"]

```

#### [§](#options)Options

You can name your arguments with a flag:

- Intent of the value is clearer

- Order doesn’t matter

To specify the flags for an argument, you can use [`#[arg(short = 'n')]`](../../struct.Arg.html#method.short) and/or
[`#[arg(long = "name")]`](../../struct.Arg.html#method.long) attributes on a field.  When no value is given (e.g.
`#[arg(short)]`), the flag is inferred from the field’s name.

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    name: String,
}

fn main() {
    let cli = Cli::parse();

    println!("name: {:?}", cli.name);
}
```

```
$ 03_02_option_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_02_option_derive[EXE] --name <NAME>

Options:
  -n, --name <NAME>  
  -h, --help         Print help
  -V, --version      Print version

$ 03_02_option_derive
? 2
error: the following required arguments were not provided:
  --name <NAME>

Usage: 03_02_option_derive[EXE] --name <NAME>

For more information, try '--help'.

$ 03_02_option_derive --name bob
name: "bob"

$ 03_02_option_derive --name=bob
name: "bob"

$ 03_02_option_derive -n bob
name: "bob"

$ 03_02_option_derive -n=bob
name: "bob"

$ 03_02_option_derive -nbob
name: "bob"

```

Note that the [default `ArgAction` is `Set`](../index.html#arg-types).  To
accept multiple occurrences, override the [action](../../struct.Arg.html#method.action) with [`Append`](../../enum.ArgAction.html#variant.Append) via `Vec`:

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    name: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    println!("name: {:?}", cli.name);
}
```

```
$ 03_02_option_mult_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_02_option_mult_derive[EXE] [OPTIONS]

Options:
  -n, --name <NAME>  
  -h, --help         Print help
  -V, --version      Print version

$ 03_02_option_mult_derive
name: []

$ 03_02_option_mult_derive --name bob
name: ["bob"]

$ 03_02_option_mult_derive --name bob --name john
name: ["bob", "john"]

$ 03_02_option_mult_derive --name bob --name=john -n tom -n=chris -nsteve
name: ["bob", "john", "tom", "chris", "steve"]

```

#### [§](#flags)Flags

Flags can also be switches that can be on/off:

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    let cli = Cli::parse();

    println!("verbose: {:?}", cli.verbose);
}
```

```
$ 03_01_flag_bool_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_01_flag_bool_derive[EXE] [OPTIONS]

Options:
  -v, --verbose  
  -h, --help     Print help
  -V, --version  Print version

$ 03_01_flag_bool_derive
verbose: false

$ 03_01_flag_bool_derive --verbose
verbose: true

$ 03_01_flag_bool_derive --verbose --verbose
? failed
error: the argument '--verbose' cannot be used multiple times

Usage: 03_01_flag_bool_derive[EXE] [OPTIONS]

For more information, try '--help'.

```

Note that the default `ArgAction` for a `bool` field is
`SetTrue`.  To accept multiple flags, override the [action](../../struct.Arg.html#method.action) with
[`Count`](../../enum.ArgAction.html#variant.Count):

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() {
    let cli = Cli::parse();

    println!("verbose: {:?}", cli.verbose);
}
```

```
$ 03_01_flag_count_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_01_flag_count_derive[EXE] [OPTIONS]

Options:
  -v, --verbose...  
  -h, --help        Print help
  -V, --version     Print version

$ 03_01_flag_count_derive
verbose: 0

$ 03_01_flag_count_derive --verbose
verbose: 1

$ 03_01_flag_count_derive --verbose --verbose
verbose: 2

```

This also shows that any[`Arg`](../../trait.Args.html) method may be used as an attribute.

#### [§](#optional)Optional

By default, arguments are assumed to be [`required`](../../struct.Arg.html#method.required).
To make an argument optional, wrap the field’s type in `Option`:

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    name: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    println!("name: {:?}", cli.name);
}
```

```
$ 03_06_optional_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_06_optional_derive[EXE] [NAME]

Arguments:
  [NAME]  

Options:
  -h, --help     Print help
  -V, --version  Print version

$ 03_06_optional_derive
name: None

$ 03_06_optional_derive bob
name: Some("bob")

```

#### [§](#defaults)Defaults

We’ve previously showed that arguments can be [`required`](../../struct.Arg.html#method.required) or optional.
When optional, you work with a `Option` and can `unwrap_or`.  Alternatively, you can
set [`#[arg(default_value_t)]`](../index.html#arg-attributes).

```
use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(default_value_t = 2020)]
    port: u16,
}

fn main() {
    let cli = Cli::parse();

    println!("port: {:?}", cli.port);
}
```

```
$ 03_05_default_values_derive --help
A simple to use, efficient, and full-featured Command Line Argument Parser

Usage: 03_05_default_values_derive[EXE] [PORT]

Arguments:
  [PORT]  [default: 2020]

Options:
  -h, --help     Print help
  -V, --version  Print version

$ 03_05_default_values_derive
port: 2020

$ 03_05_default_values_derive 22
port: 22

```

#### [§](#subcommands)Subcommands

Subcommands are derived with `#[derive(Subcommand)]` and be added via
[`#[command(subcommand)]` attribute](../index.html#command-attributes) on the field using that type.
Each instance of a [Subcommand](../../trait.Subcommand.html) can have its own version, author(s), Args,
and even its own subcommands.

```
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Adds files to myapp
    Add { name: Option<String> },
}

fn main() {
    let cli = Cli::parse();

    // You can check for the existence of subcommands, and if found use their
    // matches just as you would the top level cmd
    match &cli.command {
        Commands::Add { name } => {
            println!("'myapp add' was used, name is: {name:?}");
        }
    }
}
```

We used a struct-variant to define the `add` subcommand.
Alternatively, you can use a struct for your subcommand’s arguments:

```
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Adds files to myapp
    Add(AddArgs),
}

#[derive(Args)]
struct AddArgs {
    name: Option<String>,
}

fn main() {
    let cli = C

... [Content truncated]