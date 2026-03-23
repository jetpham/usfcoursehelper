use anyhow::{anyhow, Context, Result};
use clap::Parser;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime};
use csv::Writer;
use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const BASE_URL: &str = "https://reg-prod.ec.usfca.edu/StudentRegistrationSsb";
const PAGE_SIZE: usize = 100;

#[derive(Debug, Deserialize, Clone)]
struct Term {
    code: String,
    description: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct Faculty {
    display_name: Option<String>,
    email_address: Option<String>,
    primary_indicator: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct MeetingTime {
    begin_time: Option<String>,
    building_description: Option<String>,
    campus_description: Option<String>,
    end_date: Option<String>,
    end_time: Option<String>,
    friday: Option<bool>,
    meeting_type_description: Option<String>,
    monday: Option<bool>,
    room: Option<String>,
    saturday: Option<bool>,
    start_date: Option<String>,
    sunday: Option<bool>,
    thursday: Option<bool>,
    tuesday: Option<bool>,
    wednesday: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct Meeting {
    #[serde(default)]
    faculty: Vec<Faculty>,
    meeting_time: Option<MeetingTime>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct Course {
    term: Option<String>,
    term_desc: Option<String>,
    course_reference_number: Option<String>,
    part_of_term: Option<String>,
    course_number: Option<String>,
    subject: Option<String>,
    subject_description: Option<String>,
    sequence_number: Option<String>,
    campus_description: Option<String>,
    schedule_type_description: Option<String>,
    course_title: Option<String>,
    credit_hours: Option<f32>,
    enrollment: Option<i32>,
    maximum_enrollment: Option<i32>,
    seats_available: Option<i32>,
    wait_count: Option<i32>,
    instructional_method_description: Option<String>,
    subject_course: Option<String>,
    #[serde(default)]
    faculty: Vec<Faculty>,
    #[serde(default)]
    meetings_faculty: Vec<Meeting>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ApiResponse {
    #[serde(default)]
    total_count: Option<usize>,
    #[serde(default)]
    data: Vec<Course>,
}

#[derive(Debug, Parser)]
#[command(name = "usfcoursehelper")]
#[command(about = "Scrape USF course sections into CSV")]
struct ScrapeConfig {
    #[arg(long)]
    list_terms: bool,

    #[arg(short, long, env = "TERM_CODE")]
    term_code: Option<String>,

    #[arg(short, long, env = "SUBJECT_CODE")]
    subject: Option<String>,

    #[arg(long, env = "OUTPUT_CSV", num_args = 0..=1, default_missing_value = ".")]
    csv: Option<String>,

    #[arg(long, env = "CALENDAR_DIR", num_args = 0..=1, default_missing_value = ".")]
    calendar: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = read_config();
    let client = build_client()?;
    let setup_progress = spinner("Loading USF term selection page");

    let term_page_html = client
        .get(format!("{BASE_URL}/ssb/term/termSelection?mode=search"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await
        .context("failed to read USF term selection page")?;

    let synchronizer_token = extract_synchronizer_token(&term_page_html)?;
    let unique_session_id = build_unique_session_id();
    let headers = build_ajax_headers(&synchronizer_token)?;

    setup_progress.set_message("Fetching available USF terms");
    let terms = fetch_terms(&client, &headers)
        .await
        .context("failed to fetch available USF terms")?;
    if config.list_terms || config.term_code.is_none() {
        setup_progress.finish_and_clear();
        print_terms(&terms);
        return Ok(());
    }

    let requested_term_code = config.term_code.as_deref().unwrap_or_default();

    let selected_term = select_term(&terms, requested_term_code)?;
    setup_progress.set_message(format!(
        "Preparing session for {} ({})",
        selected_term.description, selected_term.code
    ));

    save_term(&client, &headers, &selected_term.code, &unique_session_id)
        .await
        .with_context(|| format!("failed to save term {}", selected_term.code))?;
    transition_to_search(
        &client,
        &headers,
        &selected_term.code,
        &unique_session_id,
    )
    .await
    .with_context(|| format!("failed to start search session for {}", selected_term.code))?;

    setup_progress.finish_with_message(format!(
        "Session ready for {} ({})",
        selected_term.description, selected_term.code
    ));

    let fetch_progress = progress_bar(&format!(
        "Fetching sections for {}",
        selected_term.description
    ));

    let courses = fetch_all_courses(
        &client,
        &headers,
        &selected_term.code,
        &unique_session_id,
        config.subject.as_deref(),
        &fetch_progress,
    )
    .await
    .with_context(|| format!("failed while downloading sections for {}", selected_term.code))?;

    fetch_progress.finish_with_message(format!(
        "Fetched {} section rows{} for {}",
        courses.len(),
        config
            .subject
            .as_deref()
            .map(|subject| format!(" in subject {subject}"))
            .unwrap_or_default(),
        selected_term.description
    ));

    if let Some(csv_dir) = config.csv.as_deref() {
        let csv_path = resolve_csv_path(csv_dir);
        let csv_progress = spinner(&format!("Writing CSV to {}", csv_path.display()));
        write_csv(&csv_path, &courses)?;
        csv_progress.finish_with_message(format!("CSV written to {}", csv_path.display()));

        println!(
            "Wrote {} sections{} for {} ({}) to {}",
            courses.len(),
            config
                .subject
                .as_deref()
                .map(|subject| format!(" for subject {subject}"))
                .unwrap_or_default(),
            selected_term.description,
            selected_term.code,
            csv_path.display()
        );
    }

    if let Some(calendar_dir) = config.calendar.as_deref() {
        let calendar_dir = resolve_calendar_dir(calendar_dir);
        let calendar_progress = spinner(&format!(
            "Writing subject calendars to {}",
            calendar_dir.display()
        ));
        write_subject_calendars(&calendar_dir, &selected_term, &courses)?;
        calendar_progress.finish_with_message(format!(
            "Subject calendars written to {}",
            calendar_dir.display()
        ));
        println!(
            "Wrote subject calendars for {} ({}) to {}",
            selected_term.description,
            selected_term.code,
            calendar_dir.display()
        );
    }

    if config.csv.is_none() && config.calendar.is_none() {
        println!(
            "Fetched {} sections{} for {} ({}). No files written. Use `--csv`, `--calendar`, or both.",
            courses.len(),
            config
                .subject
                .as_deref()
                .map(|subject| format!(" for subject {subject}"))
                .unwrap_or_default(),
            selected_term.description,
            selected_term.code,
        );
        println!("Examples:");
        println!("  cargo run -- --term-code {} --csv", selected_term.code);
        println!("  cargo run -- --term-code {} --calendar", selected_term.code);
        println!(
            "  cargo run -- --term-code {} --csv --calendar",
            selected_term.code
        );
    }

    Ok(())
}

fn read_config() -> ScrapeConfig {
    let mut config = ScrapeConfig::parse();
    config.subject = config
        .subject
        .take()
        .map(|subject| subject.trim().to_uppercase())
        .filter(|subject| !subject.is_empty());
    config.term_code = config
        .term_code
        .take()
        .map(|term| term.trim().to_string())
        .filter(|term| !term.is_empty());
    config.csv = config
        .csv
        .take()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());
    config.calendar = config
        .calendar
        .take()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());
    config
}

fn resolve_csv_path(raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.extension().and_then(|ext| ext.to_str()) == Some("csv") {
        path.to_path_buf()
    } else {
        path.join("output.csv")
    }
}

fn resolve_calendar_dir(raw: &str) -> PathBuf {
    PathBuf::from(raw)
}

fn build_client() -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (X11; Linux x86_64; rv:136.0) Gecko/20100101 Firefox/136.0",
        ),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/javascript, */*; q=0.01"),
    );

    reqwest::Client::builder()
        .cookie_store(true)
        .default_headers(headers)
        .build()
        .context("failed to build HTTP client")
}

fn extract_synchronizer_token(html: &str) -> Result<String> {
    let regex = Regex::new(r#"meta name="synchronizerToken" content="([^"]+)""#)?;
    let captures = regex
        .captures(html)
        .ok_or_else(|| anyhow!("could not find synchronizer token in term page"))?;

    Ok(captures[1].to_string())
}

fn build_unique_session_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("rs{millis}")
}

fn build_ajax_headers(synchronizer_token: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Synchronizer-Token",
        HeaderValue::from_str(synchronizer_token)?,
    );
    headers.insert(
        "X-Requested-With",
        HeaderValue::from_static("XMLHttpRequest"),
    );
    headers.insert(
        "Referer",
        HeaderValue::from_static(
            "https://reg-prod.ec.usfca.edu/StudentRegistrationSsb/ssb/classSearch/classSearch",
        ),
    );
    Ok(headers)
}

async fn fetch_terms(client: &reqwest::Client, headers: &HeaderMap) -> Result<Vec<Term>> {
    let terms = client
        .get(format!("{BASE_URL}/ssb/classSearch/getTerms"))
        .headers(headers.clone())
        .query(&[("searchTerm", ""), ("offset", "1"), ("max", "20")])
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<Term>>()
        .await?;

    if terms.is_empty() {
        return Err(anyhow!("USF returned no searchable terms"));
    }

    Ok(terms)
}

fn select_term(terms: &[Term], requested_code: &str) -> Result<Term> {
    terms
        .iter()
        .find(|term| term.code == requested_code)
        .cloned()
        .ok_or_else(|| anyhow!("term code {requested_code} was not returned by USF"))
}

fn select_current_term(terms: &[Term]) -> Option<Term> {
    let today = Local::now().date_naive();
    let current_year = today.year();
    let current_season = match today.month() {
        1 => "Intersession",
        2..=5 => "Spring",
        6..=8 => "Summer",
        _ => "Fall",
    };

    terms
        .iter()
        .find(|term| {
            !term.description.contains("View Only")
                && term.description.contains(current_season)
                && term.description.contains(&current_year.to_string())
        })
        .cloned()
        .or_else(|| {
            terms
                .iter()
                .find(|term| {
                    !term.description.contains("View Only")
                        && term.description.contains("Spring")
                        && term.description.contains(&current_year.to_string())
                })
                .cloned()
        })
}

fn print_terms(terms: &[Term]) {
    let current_term = select_current_term(terms);
    let mut grouped_terms: BTreeMap<String, Vec<&Term>> = BTreeMap::new();

    for term in terms {
        grouped_terms
            .entry(term_year(term).unwrap_or_else(|| "Unknown".to_string()))
            .or_default()
            .push(term);
    }

    println!("Available terms:\n");
    for (year, year_terms) in grouped_terms.iter().rev() {
        println!("{year}");
        for term in year_terms {
            let marker = if current_term
                .as_ref()
                .map(|current| current.code.as_str())
                == Some(term.code.as_str())
            {
                "*"
            } else {
                "-"
            };
            let current_label = if marker == "*" { "  <-- current" } else { "" };
            println!("  {marker} {}  {}{}", term.code, term.description, current_label);
        }
        println!();
    }

    println!("Run with `--term-code <TERM>` to scrape a term.");
    println!("Use `--list-terms` to print this list explicitly.");
}

fn spinner(message: &str) -> ProgressBar {
    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::with_template("{spinner:.green} {msg}")
            .expect("valid spinner template"),
    );
    progress.set_message(message.to_string());
    progress.enable_steady_tick(std::time::Duration::from_millis(100));
    progress
}

fn progress_bar(message: &str) -> ProgressBar {
    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::with_template("{spinner:.green} {msg}")
            .expect("valid progress template"),
    );
    progress.set_message(message.to_string());
    progress.enable_steady_tick(std::time::Duration::from_millis(100));
    progress
}

fn progress_style() -> ProgressStyle {
    ProgressStyle::with_template("{bar:40.cyan/blue} {pos:>4}/{len:4} {msg}")
        .expect("valid progress bar template")
        .progress_chars("##-")
}

fn term_year(term: &Term) -> Option<String> {
    term.description
        .chars()
        .collect::<String>()
        .split_whitespace()
        .find(|part| part.len() == 4 && part.chars().all(|c| c.is_ascii_digit()))
        .map(|year| year.to_string())
}

async fn save_term(
    client: &reqwest::Client,
    headers: &HeaderMap,
    term_code: &str,
    unique_session_id: &str,
) -> Result<()> {
    client
        .get(format!("{BASE_URL}/ssb/term/saveTerm"))
        .headers(headers.clone())
        .query(&[
            ("mode", "search"),
            ("term", term_code),
            ("uniqueSessionId", unique_session_id),
        ])
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

async fn transition_to_search(
    client: &reqwest::Client,
    headers: &HeaderMap,
    term_code: &str,
    unique_session_id: &str,
) -> Result<()> {
    client
        .post(format!("{BASE_URL}/ssb/term/search"))
        .headers(headers.clone())
        .query(&[("mode", "search")])
        .form(&[
            ("term", term_code),
            ("studyPath", ""),
            ("studyPathText", ""),
            ("student", ""),
            ("altPin", ""),
            ("stu_pin", ""),
            ("holdPassword", ""),
            ("startDatepicker", ""),
            ("endDatepicker", ""),
            ("uniqueSessionId", unique_session_id),
        ])
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

async fn fetch_all_courses(
    client: &reqwest::Client,
    headers: &HeaderMap,
    term_code: &str,
    unique_session_id: &str,
    subject: Option<&str>,
    progress: &ProgressBar,
) -> Result<Vec<Course>> {
    let mut courses = Vec::new();
    let mut page_offset = 0usize;
    let mut total_count = None;

    loop {
        let response = client
            .get(format!("{BASE_URL}/ssb/searchResults/searchResults"))
            .headers(headers.clone())
            .query(&[
                ("txt_term", term_code),
                ("startDatepicker", ""),
                ("endDatepicker", ""),
                ("pageOffset", &page_offset.to_string()),
                ("pageMaxSize", &PAGE_SIZE.to_string()),
                ("sortColumn", "subjectDescription"),
                ("sortDirection", "asc"),
                ("uniqueSessionId", unique_session_id),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<ApiResponse>()
            .await?;

        if total_count.is_none() {
            total_count = response.total_count;
            if let Some(total) = total_count {
                progress.set_length(total as u64);
                progress.set_style(progress_style());
            }
        }

        let batch_count = response.data.len();
        progress.inc(batch_count as u64);
        progress.set_message(format!(
            "Fetching sections for term {term_code} (offset {page_offset})"
        ));
        courses.extend(
            response
                .data
                .into_iter()
                .filter(|course| match subject {
                    Some(subject_code) => course.subject.as_deref() == Some(subject_code),
                    None => true,
                }),
        );

        if batch_count < PAGE_SIZE {
            break;
        }

        page_offset += PAGE_SIZE;

        if let Some(total) = total_count {
            if page_offset >= total {
                break;
            }
        }
    }

    Ok(courses)
}

fn write_csv(output_path: &Path, courses: &[Course]) -> Result<()> {
    if let Some(parent) = output_path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create CSV output directory at {}", parent.display())
        })?;
    }

    let file = File::create(output_path)
        .with_context(|| format!("failed to create CSV output at {}", output_path.display()))?;
    let mut writer = Writer::from_writer(file);

    writer.write_record([
        "term_code",
        "term_name",
        "crn",
        "part_of_term",
        "subject",
        "course_number",
        "section",
        "subject_course",
        "class_name",
        "class_label",
        "primary_instructor",
        "primary_instructor_email",
        "instructor_emails",
        "all_instructors",
        "meeting_days",
        "meeting_time",
        "location",
        "start_date",
        "end_date",
        "meeting_type",
        "meeting_details",
        "additional_meetings",
        "schedule_type",
        "instructional_method",
        "credit_hours",
        "campus",
        "enrollment",
        "capacity",
        "seats_available",
        "wait_count",
    ])?;

    for course in courses {
        let professor_pairs = collect_professors(course);
        let meetings = collect_meetings(course);
        let primary_instructor = professor_pairs
            .iter()
            .find(|(name, _)| name.contains("(primary)"))
            .cloned()
            .or_else(|| professor_pairs.first().cloned())
            .unwrap_or_default();
        let meeting_details = join_strings(meetings.iter().map(|meeting| meeting.summary.clone()));
        let primary_instructor_name = if primary_instructor.0.is_empty() {
            String::new()
        } else {
            primary_instructor.0.clone()
        };
        let primary_instructor_email = primary_instructor
            .1
            .clone()
            .unwrap_or_default();
        let meeting_bundle = build_section_meeting_bundle(&meetings);
        let class_name = text(course.course_title.as_deref());
        let subject = text(course.subject.as_deref());
        let course_number = text(course.course_number.as_deref());
        let section = text(course.sequence_number.as_deref());
        let class_label = [subject.clone(), course_number.clone(), section.clone(), class_name.clone()]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        writer.write_record([
            text(course.term.as_deref()),
            text(course.term_desc.as_deref()),
            text(course.course_reference_number.as_deref()),
            text(course.part_of_term.as_deref()),
            text(course.subject.as_deref()),
            text(course.course_number.as_deref()),
            text(course.sequence_number.as_deref()),
            text(course.subject_course.as_deref()),
            class_name,
            class_label,
            primary_instructor_name,
            primary_instructor_email,
            join_strings(professor_pairs.iter().filter_map(|(_, email)| email.clone())),
            join_strings(professor_pairs.iter().map(|(name, _)| name.clone())),
            meeting_bundle.meeting_days,
            meeting_bundle.meeting_time,
            meeting_bundle.location,
            meeting_bundle.start_date,
            meeting_bundle.end_date,
            meeting_bundle.meeting_type,
            meeting_details,
            meeting_bundle.additional_meetings,
            text(course.schedule_type_description.as_deref()),
            text(course.instructional_method_description.as_deref()),
            format_optional_float(course.credit_hours),
            text(course.campus_description.as_deref()),
            format_optional_i32(course.enrollment),
            format_optional_i32(course.maximum_enrollment),
            format_optional_i32(course.seats_available),
            format_optional_i32(course.wait_count),
        ])?;
    }

    writer.flush()?;
    Ok(())
}

fn write_subject_calendars(output_dir: &Path, term: &Term, courses: &[Course]) -> Result<()> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create calendar output directory at {}", output_dir.display()))?;

    clear_term_calendars(output_dir, &term.code)?;

    let mut calendars: BTreeMap<String, SubjectCalendar> = BTreeMap::new();
    let generated_at = format_ics_timestamp(Local::now().naive_local());

    for course in courses {
        let subject_code = text(course.subject.as_deref());
        if subject_code.is_empty() {
            continue;
        }

        let subject_name = text(course.subject_description.as_deref());
        let calendar = calendars
            .entry(subject_code.clone())
            .or_insert_with(|| SubjectCalendar {
                subject_code: subject_code.clone(),
                subject_name: subject_name.clone(),
                events: Vec::new(),
            });

        if calendar.subject_name.is_empty() && !subject_name.is_empty() {
            calendar.subject_name = subject_name;
        }

        for event in build_calendar_events(term, course, &generated_at) {
            calendar.events.push(event);
        }
    }

    for (_, calendar_data) in calendars {
        let mut calendar = Vec::new();
        calendar.push("BEGIN:VCALENDAR".to_string());
        calendar.push("VERSION:2.0".to_string());
        calendar.push("PRODID:-//usfcoursehelper//USF Subject Calendar//EN".to_string());
        calendar.push("CALSCALE:GREGORIAN".to_string());
        let display_subject = if calendar_data.subject_name.is_empty() {
            calendar_data.subject_code.clone()
        } else {
            calendar_data.subject_name.clone()
        };
        calendar.push(format!(
            "X-WR-CALNAME:{} {}",
            escape_ics_text(&display_subject),
            escape_ics_text(&term.description)
        ));
        calendar.extend(calendar_data.events);
        calendar.push("END:VCALENDAR".to_string());

        let file_name = format!(
            "{}-{}.ics",
            sanitize_filename(&calendar_data.subject_code),
            term.code
        );
        let file_path = output_dir.join(file_name);
        fs::write(&file_path, calendar.join("\r\n") + "\r\n")
            .with_context(|| format!("failed to write subject calendar to {}", file_path.display()))?;
    }

    Ok(())
}

fn clear_term_calendars(output_dir: &Path, term_code: &str) -> Result<()> {
    for entry in fs::read_dir(output_dir)
        .with_context(|| format!("failed to read calendar output directory {}", output_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();

        if path.is_file() && file_name.ends_with(&format!("-{term_code}.ics")) {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove old calendar {}", path.display()))?;
        }
    }

    Ok(())
}

#[derive(Debug, Default)]
struct SubjectCalendar {
    subject_code: String,
    subject_name: String,
    events: Vec<String>,
}

fn build_calendar_events(term: &Term, course: &Course, generated_at: &str) -> Vec<String> {
    let crn = text(course.course_reference_number.as_deref());
    let subject = text(course.subject.as_deref());
    let course_number = text(course.course_number.as_deref());
    let section = text(course.sequence_number.as_deref());
    let class_name = text(course.course_title.as_deref());
    let title = [subject.clone(), course_number, section, class_name]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    let instructor = collect_professors(course)
        .into_iter()
        .find(|(name, _)| name.contains("(primary)"))
        .or_else(|| collect_professors(course).into_iter().next())
        .unwrap_or_default();
    let description = build_event_description(course, &instructor.0, instructor.1.as_deref());

    course
        .meetings_faculty
        .iter()
        .filter_map(|meeting| {
            let meeting_time = meeting.meeting_time.as_ref()?;
            let days = meeting_days(meeting_time);
            if days.is_empty() {
                return None;
            }

            let begin = parse_banner_time(meeting_time.begin_time.as_deref()?)?;
            let end = parse_banner_time(meeting_time.end_time.as_deref()?)?;
            let start_date = parse_banner_date(meeting_time.start_date.as_deref()?)?;
            let end_date = parse_banner_date(meeting_time.end_date.as_deref()?)?;
            let location = meeting_location(meeting_time);
            let meeting_type = text(meeting_time.meeting_type_description.as_deref());
            let uid = format!(
                "{}-{}-{}-{}@usfcoursehelper",
                term.code,
                crn,
                start_date.format("%Y%m%d"),
                meeting_type.replace(' ', "-").to_ascii_lowercase()
            );

            Some(build_ics_event(
                &uid,
                generated_at,
                &title,
                &description,
                &location,
                start_date,
                end_date,
                begin,
                end,
                &days,
            ))
        })
        .collect()
}

fn build_event_description(course: &Course, primary_instructor: &str, primary_email: Option<&str>) -> String {
    let mut parts = Vec::new();

    let subject_course = text(course.subject_course.as_deref());
    let crn = text(course.course_reference_number.as_deref());
    let schedule_type = text(course.schedule_type_description.as_deref());
    let method = text(course.instructional_method_description.as_deref());
    let campus = text(course.campus_description.as_deref());

    if !subject_course.is_empty() {
        parts.push(format!("Course: {subject_course}"));
    }
    if !crn.is_empty() {
        parts.push(format!("CRN: {crn}"));
    }
    if !primary_instructor.is_empty() {
        parts.push(format!("Instructor: {primary_instructor}"));
    }
    if let Some(email) = primary_email.filter(|email| !email.is_empty()) {
        parts.push(format!("Email: {email}"));
    }
    if !schedule_type.is_empty() {
        parts.push(format!("Schedule Type: {schedule_type}"));
    }
    if !method.is_empty() {
        parts.push(format!("Instructional Method: {method}"));
    }
    if !campus.is_empty() {
        parts.push(format!("Campus: {campus}"));
    }

    parts.join(" | ")
}

#[allow(clippy::too_many_arguments)]
fn build_ics_event(
    uid: &str,
    generated_at: &str,
    title: &str,
    description: &str,
    location: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
    start_time: NaiveTime,
    end_time: NaiveTime,
    meeting_days: &str,
) -> String {
    let dtstart = format_ics_timestamp(NaiveDateTime::new(start_date, start_time));
    let dtend = format_ics_timestamp(NaiveDateTime::new(start_date, end_time));
    let until = format!("{}T235959", end_date.format("%Y%m%d"));
    let byday = meeting_days_to_rrule(meeting_days);

    [
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", escape_ics_text(uid)),
        format!("DTSTAMP:{}", generated_at),
        format!("SUMMARY:{}", escape_ics_text(title)),
        format!("DESCRIPTION:{}", escape_ics_text(description)),
        format!("LOCATION:{}", escape_ics_text(location)),
        format!("DTSTART:{}", dtstart),
        format!("DTEND:{}", dtend),
        format!("RRULE:FREQ=WEEKLY;BYDAY={byday};UNTIL={until}"),
        "END:VEVENT".to_string(),
    ]
    .join("\r\n")
}

fn parse_banner_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%m/%d/%Y").ok()
}

fn parse_banner_time(raw: &str) -> Option<NaiveTime> {
    if raw.len() != 4 {
        return None;
    }

    let hour = raw[0..2].parse().ok()?;
    let minute = raw[2..4].parse().ok()?;
    NaiveTime::from_hms_opt(hour, minute, 0)
}

fn meeting_days_to_rrule(days: &str) -> String {
    days.chars()
        .filter_map(|day| match day {
            'M' => Some("MO"),
            'T' => Some("TU"),
            'W' => Some("WE"),
            'R' => Some("TH"),
            'F' => Some("FR"),
            'S' => Some("SA"),
            'U' => Some("SU"),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_ics_timestamp(datetime: NaiveDateTime) -> String {
    datetime.format("%Y%m%dT%H%M%S").to_string()
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn escape_ics_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

#[derive(Debug)]
struct MeetingSummary {
    days: String,
    time_range: String,
    location: String,
    start_date: String,
    end_date: String,
    meeting_type: String,
    summary: String,
}

#[derive(Debug, Default)]
struct SectionMeetingBundle {
    meeting_days: String,
    meeting_time: String,
    location: String,
    start_date: String,
    end_date: String,
    meeting_type: String,
    additional_meetings: String,
}

fn collect_professors(course: &Course) -> Vec<(String, Option<String>)> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();

    let faculty_iter = course
        .faculty
        .iter()
        .chain(course.meetings_faculty.iter().flat_map(|meeting| meeting.faculty.iter()));

    for faculty in faculty_iter {
        let name = faculty.display_name.clone().unwrap_or_default();
        let email = faculty.email_address.clone();
        let primary = faculty.primary_indicator.unwrap_or(false);

        if name.is_empty() && email.as_deref().unwrap_or_default().is_empty() {
            continue;
        }

        let key = format!("{}|{}", name, email.clone().unwrap_or_default());
        if seen.insert(key) {
            let display = if primary && !name.is_empty() {
                format!("{name} (primary)")
            } else {
                name
            };
            result.push((display, email));
        }
    }

    result
}

fn collect_meetings(course: &Course) -> Vec<MeetingSummary> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();

    for meeting in &course.meetings_faculty {
        let Some(meeting_time) = &meeting.meeting_time else {
            continue;
        };

        let days = meeting_days(meeting_time);
        let time_range = meeting_range(meeting_time);
        let location = meeting_location(meeting_time);
        let meeting_type = text(meeting_time.meeting_type_description.as_deref());
        let start_date = text(meeting_time.start_date.as_deref());
        let end_date = text(meeting_time.end_date.as_deref());
        let date_range = if !start_date.is_empty() || !end_date.is_empty() {
            format!("{start_date} to {end_date}")
        } else {
            String::new()
        };

        let mut parts = Vec::new();
        if !meeting_type.is_empty() {
            parts.push(meeting_type.clone());
        }
        if !days.is_empty() || !time_range.is_empty() {
            let schedule = [days.clone(), time_range.clone()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !schedule.is_empty() {
                parts.push(schedule);
            }
        }
        if !location.is_empty() {
            parts.push(location.clone());
        }
        if !date_range.is_empty() {
            parts.push(date_range);
        }

        let summary = parts.join(" | ");
        if !summary.is_empty() && seen.insert(summary.clone()) {
            result.push(MeetingSummary {
                days,
                time_range,
                location,
                start_date,
                end_date,
                meeting_type,
                summary,
            });
        }
    }

    result
}

fn build_section_meeting_bundle(meetings: &[MeetingSummary]) -> SectionMeetingBundle {
    let mut regular_meetings: Vec<&MeetingSummary> = Vec::new();
    let mut additional_meetings = Vec::new();

    for meeting in meetings {
        if is_primary_class_meeting(meeting) {
            regular_meetings.push(meeting);
        } else {
            additional_meetings.push(meeting.summary.clone());
        }
    }

    let primary_source: Vec<&MeetingSummary> = if regular_meetings.is_empty() {
        meetings.iter().collect()
    } else {
        regular_meetings
    };

    SectionMeetingBundle {
        meeting_days: join_strings(primary_source.iter().map(|meeting| meeting.days.clone())),
        meeting_time: join_strings(primary_source.iter().map(|meeting| meeting.time_range.clone())),
        location: join_strings(primary_source.iter().map(|meeting| meeting.location.clone())),
        start_date: join_strings(primary_source.iter().map(|meeting| meeting.start_date.clone())),
        end_date: join_strings(primary_source.iter().map(|meeting| meeting.end_date.clone())),
        meeting_type: join_strings(primary_source.iter().map(|meeting| meeting.meeting_type.clone())),
        additional_meetings: join_strings(additional_meetings),
    }
}

fn is_primary_class_meeting(meeting: &MeetingSummary) -> bool {
    let normalized = meeting.meeting_type.to_ascii_lowercase();
    !normalized.contains("final") && !normalized.contains("exam")
}

fn meeting_days(meeting_time: &MeetingTime) -> String {
    let mut days = String::new();

    if meeting_time.monday.unwrap_or(false) {
        days.push('M');
    }
    if meeting_time.tuesday.unwrap_or(false) {
        days.push('T');
    }
    if meeting_time.wednesday.unwrap_or(false) {
        days.push('W');
    }
    if meeting_time.thursday.unwrap_or(false) {
        days.push('R');
    }
    if meeting_time.friday.unwrap_or(false) {
        days.push('F');
    }
    if meeting_time.saturday.unwrap_or(false) {
        days.push('S');
    }
    if meeting_time.sunday.unwrap_or(false) {
        days.push('U');
    }

    days
}

fn meeting_range(meeting_time: &MeetingTime) -> String {
    match (
        meeting_time.begin_time.as_deref(),
        meeting_time.end_time.as_deref(),
    ) {
        (Some(begin), Some(end)) => format!("{}-{}", format_time(begin), format_time(end)),
        (Some(begin), None) => format_time(begin),
        _ => String::new(),
    }
}

fn meeting_location(meeting_time: &MeetingTime) -> String {
    let campus = text(meeting_time.campus_description.as_deref());
    let building = text(meeting_time.building_description.as_deref());
    let room = text(meeting_time.room.as_deref());

    [campus, building, room]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

fn format_time(raw: &str) -> String {
    if raw.len() != 4 {
        return raw.to_string();
    }

    let hour = raw[0..2].parse::<u32>().unwrap_or(0);
    let minute = &raw[2..4];
    let suffix = if hour >= 12 { "PM" } else { "AM" };
    let hour12 = match hour % 12 {
        0 => 12,
        value => value,
    };

    format!("{hour12}:{minute} {suffix}")
}

fn format_optional_i32(value: Option<i32>) -> String {
    value.map(|number| number.to_string()).unwrap_or_default()
}

fn format_optional_float(value: Option<f32>) -> String {
    value
        .map(|number| {
            if (number.fract() - 0.0).abs() < f32::EPSILON {
                format!("{number:.0}")
            } else {
                number.to_string()
            }
        })
        .unwrap_or_default()
}

fn text(value: Option<&str>) -> String {
    value.unwrap_or_default().trim().to_string()
}

fn join_strings<I>(values: I) -> String
where
    I: IntoIterator<Item = String>,
{
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect::<Vec<_>>()
        .join(" ; ")
}
