# usfcoursehelper

`usfcoursehelper` pulls section data from USF's course search and turns it into files you can actually use.

Right now it can:

- list available terms
- download section data for a term
- filter by subject code
- write a CSV export
- write subject `.ics` calendars you can import into Calendar, Google Calendar, or Outlook

This is a command-line tool. It does not need a USF login for the public search endpoints it uses.

## What you need

You only need a few things:

- an internet connection
- a recent macOS, Linux, or Windows machine
- either a released binary, `nix`, or Rust/Cargo if you want to build from source

If you just want to use it, install a binary. If you want to hack on it, use the Nix dev shell or Cargo directly.

## Install

### Homebrew (macOS)

Once a tagged GitHub release exists, you can install from the release formula:

```bash
brew install https://github.com/jetpham/usfcoursehelper/releases/download/<tag>/usfcoursehelper.rb
```

Example:

```bash
brew install https://github.com/jetpham/usfcoursehelper/releases/download/v0.1.0/usfcoursehelper.rb
```

That formula installs the macOS universal binary, so it works on both Apple Silicon and Intel Macs.

### Nix

Run it without installing:

```bash
nix run github:jetpham/usfcoursehelper
```

Install it into your profile:

```bash
nix profile install github:jetpham/usfcoursehelper
```

If you already cloned the repo:

```bash
nix run . -- --list-terms
```

### Prebuilt binaries

GitHub Actions builds release artifacts for:

- macOS: Intel, Apple Silicon, and universal
- Linux: `x86_64`, `aarch64`, `armv7`
- Windows: `x86_64`, `aarch64`

Each release includes raw binaries plus `.zip`, `.tar.gz`, and `.tar.xz` archives.

### Build from source

With Cargo:

```bash
cargo build --release
```

Or with Nix:

```bash
nix build
```

## Quick start

List terms first:

```bash
usfcoursehelper --list-terms
```

Write a CSV for a specific term:

```bash
usfcoursehelper --term-code 202620 --csv
```

Write subject calendars for a term:

```bash
usfcoursehelper --term-code 202620 --calendar
```

Filter to one subject and write both outputs:

```bash
usfcoursehelper --term-code 202620 --subject CS --csv --calendar
```

## Command reference

### `--list-terms`

Prints the terms currently returned by USF.

### `--term-code <TERM>`

Chooses the term to scrape. Use `--list-terms` if you do not know the code yet.

### `--subject <SUBJECT>`

Filters results to one subject code such as `CS`, `MATH`, or `BIO`.

### `--csv [PATH]`

Writes a CSV file.

- if you pass a directory, output goes to `output.csv` inside it
- if you pass a `.csv` path, it writes exactly there
- if you use `--csv` with no value, it writes `./output.csv`

### `--calendar [PATH]`

Writes one `.ics` file per subject.

- if you pass a directory, calendar files are written there
- if you use `--calendar` with no value, files are written to `.`

## Environment variables

The same values can be provided with environment variables:

- `TERM_CODE`
- `SUBJECT_CODE`
- `OUTPUT_CSV`
- `CALENDAR_DIR`

Example:

```bash
TERM_CODE=202620 SUBJECT_CODE=CS usfcoursehelper --csv --calendar
```

## Output files

### CSV

The CSV includes fields like:

- term code and term name
- CRN
- subject, course number, and section
- class title
- instructor names and emails
- meeting days, times, and location
- enrollment and seat counts

### Calendar files

Calendar output creates one file per subject for the selected term, named like:

```text
CS-202620.ics
MATH-202620.ics
```

These events are recurring weekly events based on the meeting schedule returned by USF.

## Notes

- The tool depends on USF's public registration endpoints being available.
- If USF changes that site, scraping may need to be updated.
- Some sections may have incomplete meeting data, and those gaps will show up in the exported files too.

## Development

Enter the dev shell:

```bash
nix develop
```

Run the binary from source:

```bash
cargo run -- --list-terms
```

Run checks:

```bash
cargo check --locked
cargo test --locked
```

## Releases

The manual GitHub Actions workflow builds cross-platform binaries and packages them for release.

If you provide a `release_tag` when running the workflow, it will also:

- publish a GitHub release
- attach all built artifacts
- generate `usfcoursehelper.rb` for Homebrew installs
- attach `SHA256SUMS.txt`
