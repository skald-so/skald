//! The scripted sessions on the measured repositories: sets up the pinned tools, fetches each
//! repository at its fixed commit into a temporary directory, plays its session and prints the
//! findings as Markdown, with the machine and the git version. Progress goes to the standard
//! error. The temporary directory is deleted at the end. Findings: `docs/dev/edit-prototype.md`,
//! "Edits via Bash and generators".

use std::process::ExitCode;

use edit_overlap::replay::REPOSITORIES;
use edit_overlap::sessions::{self, Kind, Session, Step};
use edit_overlap::tools::{Install, Installer, Pin, Toolset};

/// The tools the sessions run, each pinned. The npm and pip tools are those the repositories use,
/// at the versions they name: drizzle-orm's `packageManager`, the `dprint` of its lockfile and its
/// own drizzle-kit and drizzle-orm; netbox's `packageManager` and the ruff of its `pyproject.toml`.
const TOOLSET: Toolset = Toolset {
    toolchain: &[
        Pin {
            tool: "rustc",
            version: "rustc 1.99.1 (58cf8f9c3 2026-10-13)",
        },
        Pin {
            tool: "cargo",
            version: "cargo 1.99.1 (5f94df478 2026-08-27)",
        },
        Pin {
            tool: "rustfmt",
            version: "rustfmt 1.10.0-stable (58cf8f9c35 2026-10-13)",
        },
    ],
    system: &[
        Pin {
            tool: "node",
            version: "v26.13.0",
        },
        Pin {
            tool: "npm",
            version: "11.20.0",
        },
        Pin {
            tool: "python3",
            version: "Python 3.13.2",
        },
    ],
    installs: &[
        // Diesel CLI from crates.io with the lockfile it is published with, for SQLite only, built
        // from SQLite's bundled source.
        Install {
            installer: Installer::Cargo,
            args: &[
                "diesel_cli",
                "--version",
                "2.3.14",
                "--locked",
                "--no-default-features",
                "--features",
                "sqlite-bundled",
            ],
            pins: &[Pin {
                tool: "diesel",
                version: "Version: 2.3.14",
            }],
        },
        Install {
            installer: Installer::Npm,
            args: &[
                "pnpm@10.6.3",
                "yarn@1.22.22",
                "dprint@0.46.3",
                "drizzle-kit@0.31.11",
                "drizzle-orm@0.45.4",
            ],
            pins: &[
                Pin {
                    tool: "pnpm",
                    version: "10.6.3",
                },
                Pin {
                    tool: "yarn",
                    version: "1.22.22",
                },
                Pin {
                    tool: "dprint",
                    version: "dprint 0.46.3",
                },
                Pin {
                    tool: "drizzle-kit",
                    version: "drizzle-kit: v0.31.11",
                },
                Pin {
                    tool: "drizzle-kit",
                    version: "drizzle-orm: v0.45.4",
                },
            ],
        },
        Install {
            installer: Installer::Pip,
            args: &["ruff==0.17.4"],
            pins: &[
                Pin {
                    tool: "pip",
                    version: "pip 24.3.1",
                },
                Pin {
                    tool: "ruff",
                    version: "ruff 0.17.4",
                },
            ],
        },
    ],
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
/// glob gives, a module moved and edited, the repository formatted with `dprint`, a changelog
/// deleted, a dependency removed and the lockfile rewritten offline with pnpm, a migration
/// generated with drizzle-kit from a new schema, and an edit restored.
const DRIZZLE: Session = Session {
    repository: "drizzle-orm",
    // Fills pnpm's metadata cache with the packages of its lockfile, which it resolves again, and
    // dprint's with its plugins; the lockfile stays as it is.
    setup: &[
        "cp pnpm-lock.yaml {scratch}/pnpm-lock.yaml && pnpm install --resolution-only --ignore-scripts --silent && cp {scratch}/pnpm-lock.yaml pnpm-lock.yaml",
        "dprint output-resolved-config > /dev/null",
    ],
    steps: &[
        Step::Edit {
            path: "drizzle-orm/src/logger.ts",
            old: "this.writer.write(`Query: ${query}${paramsStr}`);",
            new: "this.writer.write(\"Query (drizzle): \" + query + paramsStr);",
        },
        Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "sed -i.bak -e \"s/'DrizzleError'/\\\"DrizzleOrmError\\\"/\" drizzle-orm/src/errors.ts && rm drizzle-orm/src/errors.ts.bak",
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
            kind: Kind::Formatter,
            dir: "",
            line: "dprint fmt",
        },
        Step::Shell {
            kind: Kind::Deletion,
            dir: "",
            line: "rm changelogs/drizzle-orm/0.30.5.md",
        },
        Step::Edit {
            path: "package.json",
            old: "\t\t\"concurrently\": \"^8.2.1\",\n",
            new: "",
        },
        Step::Shell {
            kind: Kind::Lockfile,
            dir: "",
            line: "pnpm install --lockfile-only --offline --ignore-scripts --silent",
        },
        Step::Write {
            path: "integration-tests/session/schema.ts",
            contents: "import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core';\n\nexport const posts = sqliteTable('posts', {\n\tid: integer('id').primaryKey(),\n\ttitle: text('title').notNull(),\n});\n",
        },
        Step::Shell {
            kind: Kind::Generator,
            dir: "",
            line: "drizzle-kit generate --dialect sqlite --schema integration-tests/session/schema.ts --out integration-tests/session/drizzle --name init > /dev/null",
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
/// test written, a module moved, a setting renamed with `sed` in a named file, the code fixed with
/// ruff, a test deleted, a front-end dependency removed with yarn offline, a script edited and the
/// bundles built again, and an edit restored.
const NETBOX: Session = Session {
    repository: "netbox",
    // Fills yarn's cache and installs the front end's packages, which netbox ignores.
    setup: &[
        "cd netbox/project-static && yarn install --frozen-lockfile --ignore-scripts --non-interactive --silent",
    ],
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
            contents: "from utilities.string import strip_linebreaks\nfrom django.test import TestCase\n\n\nclass StripLinebreaksTest(TestCase):\n    def test_strips_both_line_endings(self):\n        self.assertEqual(strip_linebreaks('a\\r\\nb\\n'), 'ab')\n",
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
            kind: Kind::Formatter,
            dir: "",
            line: "ruff check --fix --quiet netbox/",
        },
        Step::Shell {
            kind: Kind::Deletion,
            dir: "",
            line: "rm netbox/utilities/tests/test_string.py",
        },
        Step::Shell {
            kind: Kind::Lockfile,
            dir: "netbox/project-static",
            line: "yarn remove -W typeface-roboto-mono --offline --ignore-scripts --non-interactive --silent",
        },
        Step::Edit {
            path: "netbox/project-static/src/clipboard.ts",
            old: "  for (const element of getElements('.copy-content')) {",
            new: "  for (const element of getElements('.copy-content, .copy-token')) {",
        },
        Step::Shell {
            kind: Kind::Generator,
            dir: "netbox/project-static",
            line: "yarn --silent bundle:scripts > /dev/null",
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
            sessions::run(tmp.path(), &TOOLSET, &REPOSITORIES, &sessions, progress)
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
