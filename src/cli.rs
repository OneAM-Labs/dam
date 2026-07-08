use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "dam")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(
    about = "DAM - Project Continuity Platform."
)]
#[command(
    after_help = "Run `dam <COMMAND> --help` for detailed information on a specific command."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a brand new reservoir (.dam repository) in the current directory.
    ///
    /// This establishes your tracking pool, configuration files, and staging area.
    /// It creates a hidden `.dam` directory to store historical objects and stream data.
    /// You will be interactively prompted to configure the reservoir's name and default
    /// conflict resolution rules.
    ///
    /// EXAMPLES:
    ///   dam source                  # Start the interactive initialization wizard
    ///   dam source -n "My Project"  # Initialize immediately with a specific name
    Source {
        /// Optional project name. If omitted, you will be prompted to enter one interactively.
        #[arg(long, short)]
        name: Option<String>,

        /// Upgrade the current reservoir to the latest DAM version and configuration structure.
        #[arg(long)]
        upgrade: bool,
    },

    /// View the active state of your reservoir, stream, staging area, and rule structures.
    ///
    /// The inspect command acts as your primary dashboard. By default, it shows you which
    /// stream (branch) is currently flowing, and lists all files that have been successfully
    /// collected into the staging area (meaning they are ready to be sealed).
    ///
    /// If you use the --rules flag, it transforms into a debugging tool for your rule engine,
    /// mapping out exactly where all .purities (allowlists) and .impurities (blocklists)
    /// are located in your project directory tree.
    ///
    /// EXAMPLES:
    ///   dam inspect          # View current stream and staged files
    ///   dam inspect --rules  # Scan and display the tree of all rule files in the workspace
    Inspect { 
        #[arg(long)]
        rules: bool,
        /// Optional path to a .dam file to inspect
        target: Option<String>,
    },

    /// Drain a specific section (like the staging area) to clear its contents.
    ///
    /// Primarily used to empty your active staging area. If you accidentally collected
    /// the wrong files, draining the collection resets the staging area to empty without
    /// affecting your actual workspace files or historical seals.
    ///
    /// EXAMPLES:
    ///   dam drain collection  # Empties the current staging area
    Drain {
        /// What targeting section to drain (e.g., 'collection' to empty staged files).
        target: String,
    },

    /// Permanently destroy or safely archive the active reservoir.
    ///
    /// This deletes the local `.dam` directory, wiping all indices, streams, and history.
    /// It requires explicit confirmation to prevent accidental data loss. You can optionally
    /// use the --archive flag to package the reservoir into a backup file before destroying it.
    ///
    /// EXAMPLES:
    ///   dam destroy            # Interactively prompt to delete the reservoir
    ///   dam destroy --archive  # Zip the .dam directory into a backup before deletion
    Destroy {
        /// Archive the reservoir assets as a backup instead of deleting them outright.
        #[arg(long, short)]
        archive: bool,
    },

    /// Scan and collect workspace files into the staging area using the rule engine.
    ///
    /// This command walks through the specified directory path and evaluates every file
    /// against your `.purities` (allowlist) and `.impurities` (blocklist) files.
    /// Files that pass the rule engine are added to the staging area to be sealed.
    ///
    /// CONFLICTS:
    /// If a file matches both an allowlist AND a blocklist, the conflict is resolved based
    /// on your reservoir settings. You can override this on-the-fly using --rule-priority.
    ///
    /// EXAMPLES:
    ///   dam collect .                           # Scan the entire current directory
    ///   dam collect src/                        # Scan only the 'src' folder
    ///   dam collect . --override-purities       # Scan everything, ignoring all allowlists
    ///   dam collect . --rule-priority purities  # Scan, forcing Purities to win any conflicts
    Collect {
        /// The path of the file or directory to scan and collect.
        path: String,

        /// Bypass or override normal .purities rules.
        /// If specified without a value, ignores all .purities rule files completely.
        /// If a file path is provided, reads rules ONLY from that specific file.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        override_purities: Option<String>,

        /// Bypass or override normal .impurities rules.
        /// If specified without a value, ignores all .impurities rule files completely.
        /// If a file path is provided, reads rules ONLY from that specific file.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        override_impurities: Option<String>,

        /// Set preference for which rule is superior if a file matches both.
        /// Options: 'purities' (collect the file) or 'impurities' (drop the file).
        #[arg(long, value_name = "PRIORITY")]
        rule_priority: Option<String>,
    },

    /// Seal your staged files into a permanent, immutable historical record.
    ///
    /// A Seal is the equivalent of a "commit". It takes everything currently in the staging
    /// area, hashes the contents, and saves it permanently to the timeline.
    /// Alternatively, use --list to view your most recent seals.
    ///
    /// EXAMPLES:
    ///   dam seal "Added user authentication"  # Create a new seal with a message
    ///   dam seal --list 5                     # View the 5 most recent seals
    Seal {
        /// The descriptive message for the new seal. Required unless using --list.
        message: Option<String>,

        /// List the last N seals with their timestamps, hashes, and descriptions.
        #[arg(long, short)]
        list: Option<usize>,
    },

    /// View the chronological flow of historical seals.
    ///
    /// Prints a comprehensive history of all seals (commits) made on the current stream.
    ///
    /// EXAMPLES:
    ///   dam timeline  # Print the full history log
    /// View the chronological flow of historical seals.
    Timeline {
        /// Display a graphical representation of seals, branches, and merges.
        #[arg(long, short)]
        graph: bool,
    },

    /// Manage development streams (branches) in your reservoir.
    ///
    /// Streams allow you to maintain parallel timelines of seals. You can create a new
    /// stream to test an experimental feature without affecting your `main` stream.
    ///
    /// EXAMPLES:
    ///   dam stream                 # List all available streams
    ///   dam stream experiment-ui   # Create a new stream called 'experiment-ui'
    Stream {
        #[command(subcommand)]
        command: Option<StreamCommands>,
    },

    /// Safely divert active flow (checkout) to a different development stream.
    ///
    /// Switches your active workspace to point to a different stream. Any new seals
    /// created will now be appended to the newly selected stream.
    ///
    /// EXAMPLES:
    ///   dam flow experiment-ui  # Switch to the 'experiment-ui' stream
    Flowinto {
        /// The target stream name to switch to.
        name: String,
    },

    /// Restore your workspace to match the exact state of a historical seal.
    ///
    /// This overwrites your actual files to match the snapshot taken during the specified seal.
    /// Use the --preview flag to safely see what files *would* change without actually
    /// modifying anything on your disk.
    ///
    /// EXAMPLES:
    ///   dam apply seal_001            # Overwrite files to match seal_001
    ///   dam apply seal_001 --preview  # See what would happen, but do nothing
    Apply {
        /// The seal ID (e.g. seal_001) to apply.
        seal_id: String,

        /// Preview changes and affected files without modifying workspace files.
        #[arg(long, short)]
        preview: bool,
    },

    /// Merge another stream's seals into the current active stream.
    ///
    /// By default, evaluates conflicts and registers a pending merge seal.
    /// Run with --apply to review and write changes to the workspace.
    Merge {
        /// The source stream name to merge from (required unless --apply is set).
        source: Option<String>,

        /// Interactive confirmation to apply a previously calculated pending merge.
        #[arg(long, short)]
        apply: bool,
    },

    /// Export an existing seal into shareable archive formats.
    ///
    /// Useful for sending a specific version of your project to someone else.
    /// Can export as a compressed `.seal` bundle for importing into another reservoir,
    /// or as a raw `.zip` file for standard usage.
    ///
    /// EXAMPLES:
    ///   dam export seal seal_001        # Exports as a compressed .seal bundle
    ///   dam export seal seal_001 --zip  # Exports as a standard .zip archive
    Export {
        #[command(subcommand)]
        target: ExportTarget,
    },

    /// Import a compressed .seal bundle or a raw .zip file into your reservoir.
    ///
    /// EXAMPLES:
    ///   dam import bundle.seal  # Unpack a seal bundle into the reservoir
    Import {
        /// Path to the archive file (.seal or .zip) to import.
        file: String,
    },

    /// Manage reservoir configuration settings directly or via an interactive dashboard.
    ///
    /// Allows you to tweak reservoir rules, such as conflict resolution preferences
    /// and project names.
    ///
    /// EXAMPLES:
    ///   dam settings                            # Print all current settings
    ///   dam settings --interactive              # Open the visual config dashboard
    ///   dam settings name "New Project Name"    # Explicitly update a setting
    ///   dam settings name                       # Read the value of a specific setting
    Settings {
        /// The setting key to read or update.
        key: Option<String>,

        /// The value to set. If omitted, prints the active value of the key.
        value: Option<String>,

        /// Open the interactive settings terminal dashboard.
        #[arg(long, short)]
        interactive: bool,
    },

    Sync {
        /// Stream to sync. If omitted, syncs all eligible streams interactively.
        stream: Option<String>,

        /// Action to perform: 'push' or 'pull'. If omitted, interactive diff runs.
        #[arg(long, short)]
        action: Option<String>,

        /// The cloud platform to sync with. Default is 'github'.
        #[arg(long, short)]
        platform: Option<String>,

        /// Force synchronization of the main stream without confirmation.
        #[arg(long, short)]
        force: bool,
    },
}
#[derive(Subcommand)]
pub enum StreamCommands {
    /// Create a new stream with continuity metadata
    Create {
        name: String,
        /// The base stream to branch from (defaults to the currently active stream)
        #[arg(long)]
        from: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
        #[arg(short, long)]
        owner: Option<String>,
        #[arg(short, long)]
        priority: Option<String>,
        #[arg(short, long)]
        target: Option<String>,
    },
    /// Inspect a specific stream's goals, status, and timeline (or current if omitted)
    Inspect {
        name: Option<String>,
    },
    /// Delete a stream and conditionally its associated seals
    Delete {
        name: String,
    },
    /// Manage stream goals
    Goal {
        name: String,
        #[arg(long)]
        clear: bool,
        text: Option<String>,
    },
    /// Manage stream notes
    Notes {
        name: String,
        #[arg(long)]
        clear: bool,
        text: Option<String>,
    },
    /// Manage stream description
    Description {
        name: String,
        #[arg(long)]
        clear: bool,
        text: Option<String>,
    },
    /// Manage stream target branch
    Target {
        name: String,
        #[arg(long)]
        clear: bool,
        text: Option<String>,
    }
}

#[derive(Subcommand)]
pub enum ExportTarget {
    /// Export a specific historical seal
    Seal {
        seal_id: String,
        #[arg(long)]
        zip: bool,
    },
    /// Export the current workspace as a DAM project
    Project {
        project_name: String,
        #[arg(long)]
        profile: Option<String>,
    },
}