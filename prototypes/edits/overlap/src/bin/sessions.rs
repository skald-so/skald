//! The scripted sessions on the measured repositories: sets up the pinned tools, fetches each
//! repository at its fixed commit into a temporary directory, plays its session and prints the
//! findings as Markdown, with the machine and the git version. Progress goes to the standard
//! error. The temporary directory is deleted at the end. Findings: `docs/dev/edit-prototype.md`,
//! "Edits via Bash and generators".

use std::process::ExitCode;

use edit_overlap::replay::REPOSITORIES;
use edit_overlap::sessions::{self, Kind, Session, Step};
use edit_overlap::tools::{Generator, Pins};

/// The Rust toolchain the sessions run.
const PINS: Pins = Pins {
    rustc: "rustc 1.99.1 (58cf8f9c3 2026-10-13)",
    cargo: "cargo 1.99.1 (5f94df478 2026-08-27)",
    rustfmt: "rustfmt 1.10.0-stable (58cf8f9c35 2026-10-13)",
};

/// Diesel CLI from crates.io with the lockfile it is published with, for SQLite only, built from
/// SQLite's bundled source.
const GENERATOR: Generator = Generator {
    install: &[
        "diesel_cli",
        "--version",
        "2.3.14",
        "--locked",
        "--no-default-features",
        "--features",
        "sqlite-bundled",
    ],
    binary: "diesel",
    version: "Version: 2.3.14",
};

/// The diesel example the Rust session works on.
const EXAMPLE: &str = "examples/sqlite/getting_started_step_1";

/// The session on diesel: a migration with Diesel CLI, which writes the schema, a field added with
/// `sed` and formatted with `rustfmt`, a binary moved, a test deleted, a dependency added and the
/// lockfile rewritten offline, and an edit restored.
const DIESEL: Session = Session {
    repository: "diesel",
    setup: &["cargo fetch --locked --quiet --manifest-path diesel_compile_tests/Cargo.toml"],
    steps: &[
        Step::Shell {
            kind: Kind::Generator,
            dir: EXAMPLE,
            line: "diesel migration generate --version 20261112000000 add_published_at",
        },
        Step::Write {
            path: "examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/up.sql",
            contents: "ALTER TABLE posts ADD COLUMN published_at TEXT;\n",
        },
        Step::Write {
            path: "examples/sqlite/getting_started_step_1/migrations/20261112000000_add_published_at/down.sql",
            contents: "ALTER TABLE posts DROP COLUMN published_at;\n",
        },
        Step::Shell {
            kind: Kind::Generator,
            dir: EXAMPLE,
            line: "diesel migration run --database-url {scratch}/posts.db",
        },
        Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "sed -i.bak -e 's/    pub published: bool,/    pub published: bool, pub published_at: Option<String>,/' examples/sqlite/getting_started_step_1/src/models.rs && rm examples/sqlite/getting_started_step_1/src/models.rs.bak",
        },
        Step::Shell {
            kind: Kind::Formatter,
            dir: "",
            line: "rustfmt --edition 2024 examples/sqlite/getting_started_step_1/src/lib.rs",
        },
        Step::Shell {
            kind: Kind::Move,
            dir: EXAMPLE,
            line: "mv src/bin/show_posts.rs src/bin/list_posts.rs",
        },
        Step::Edit {
            path: "examples/sqlite/getting_started_step_1/Cargo.toml",
            old: "path = \"src/bin/show_posts.rs\"",
            new: "path = \"src/bin/list_posts.rs\"",
        },
        Step::Shell {
            kind: Kind::Deletion,
            dir: EXAMPLE,
            line: "rm tests/step_1.rs",
        },
        Step::Edit {
            path: "diesel_compile_tests/Cargo.toml",
            old: "ui_test = \"0.30\"\n",
            new: "ui_test = \"0.30\"\nserde_json = \"1\"\n",
        },
        Step::Shell {
            kind: Kind::Lockfile,
            dir: "",
            line: "cargo update --offline --quiet --workspace --manifest-path diesel_compile_tests/Cargo.toml",
        },
        Step::Edit {
            path: "examples/sqlite/getting_started_step_1/README.md",
            old: "using SQLite instead of Postgresql",
            new: "using SQLite instead of PostgreSQL",
        },
        Step::Git {
            kind: Kind::Restore,
            args: &[
                "checkout",
                "--",
                "examples/sqlite/getting_started_step_1/README.md",
            ],
        },
    ],
};

/// The session on drizzle-orm: an edit, a class renamed with `sed` in a named file and in files a
/// glob gives, a module moved and edited, a changelog deleted, and an edit restored.
const DRIZZLE: Session = Session {
    repository: "drizzle-orm",
    setup: &[],
    steps: &[
        Step::Edit {
            path: "drizzle-orm/src/logger.ts",
            old: "this.writer.write(`Query: ${query}${paramsStr}`);",
            new: "this.writer.write(`Query (drizzle): ${query}${paramsStr}`);",
        },
        Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "sed -i.bak -e \"s/'DrizzleError'/'DrizzleOrmError'/\" drizzle-orm/src/errors.ts && rm drizzle-orm/src/errors.ts.bak",
        },
        Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "sed -i.bak -e 's/DrizzleError/DrizzleOrmError/g' drizzle-orm/src/*-core/dialect.ts && rm drizzle-orm/src/*-core/dialect.ts.bak",
        },
        Step::Shell {
            kind: Kind::Move,
            dir: "drizzle-orm/src",
            line: "mv casing.ts casing-cache.ts",
        },
        Step::Edit {
            path: "drizzle-orm/src/casing-cache.ts",
            old: "export function toSnakeCase(input: string) {",
            new: "export function toSnakeCase(input: string): string {",
        },
        Step::Shell {
            kind: Kind::Deletion,
            dir: "",
            line: "rm changelogs/drizzle-orm/0.30.5.md",
        },
        Step::Edit {
            path: "drizzle-orm/src/utils.ts",
            old: "export function haveSameKeys(left: Record<string, unknown>, right: Record<string, unknown>) {",
            new: "export function haveSameKeys(left: Record<string, unknown>, right: Record<string, unknown>): boolean {",
        },
        Step::Git {
            kind: Kind::Restore,
            args: &["checkout", "--", "drizzle-orm/src/utils.ts"],
        },
    ],
};

/// The session on netbox: a function renamed by an edit and by `sed` on the files `grep` finds, a
/// test written, a module moved, a setting renamed with `sed` in a named file, a test deleted, and
/// an edit restored.
const NETBOX: Session = Session {
    repository: "netbox",
    setup: &[],
    steps: &[
        Step::Edit {
            path: "netbox/utilities/string.py",
            old: "def remove_linebreaks(value):",
            new: "def strip_linebreaks(value):",
        },
        Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "grep -rl remove_linebreaks netbox | xargs sed -i.bak -e 's/remove_linebreaks/strip_linebreaks/g' && find netbox -name '*.bak' -delete",
        },
        Step::Write {
            path: "netbox/utilities/tests/test_linebreaks.py",
            contents: "from django.test import TestCase\n\nfrom utilities.string import strip_linebreaks\n\n\nclass StripLinebreaksTest(TestCase):\n    def test_strips_both_line_endings(self):\n        self.assertEqual(strip_linebreaks('a\\r\\nb\\n'), 'ab')\n",
        },
        Step::Shell {
            kind: Kind::Move,
            dir: "netbox/utilities",
            line: "mv socks.py proxy_socks.py",
        },
        Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "sed -i.bak -e 's/trailing_slash/with_trailing_slash/g' netbox/netbox/settings.py && rm netbox/netbox/settings.py.bak",
        },
        Step::Shell {
            kind: Kind::Deletion,
            dir: "",
            line: "rm netbox/utilities/tests/test_string.py",
        },
        Step::Edit {
            path: "README.md",
            old: "The cornerstone of every automated network",
            new: "The cornerstone of every automated network.",
        },
        Step::Git {
            kind: Kind::Restore,
            args: &["checkout", "--", "README.md"],
        },
    ],
};

fn main() -> ExitCode {
    let progress = |repository: &str, steps| eprintln!("sessions: {repository} step {steps}");
    let ran = tempfile::TempDir::new()
        .map_err(Into::into)
        .and_then(|tmp| {
            let sessions = [DIESEL, DRIZZLE, NETBOX];
            sessions::run(
                tmp.path(),
                &PINS,
                &GENERATOR,
                &REPOSITORIES,
                &sessions,
                progress,
            )
        });
    match ran {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("sessions: {error}");
            ExitCode::FAILURE
        }
    }
}
