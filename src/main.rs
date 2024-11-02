use csv::Writer;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Faculty {
    banner_id: Option<String>,
    category: Option<String>,
    class: Option<String>,
    course_reference_number: Option<String>,
    display_name: Option<String>,
    email_address: Option<String>,
    primary_indicator: Option<bool>,
    term: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct MeetingTime {
    begin_time: Option<String>,
    building: Option<String>,
    building_description: Option<String>,
    campus: Option<String>,
    campus_description: Option<String>,
    category: Option<String>,
    class: Option<String>,
    course_reference_number: Option<String>,
    credit_hour_session: Option<f32>,
    end_date: Option<String>,
    end_time: Option<String>,
    friday: Option<bool>,
    hours_week: Option<f32>,
    meeting_schedule_type: Option<String>,
    meeting_type: Option<String>,
    meeting_type_description: Option<String>,
    monday: Option<bool>,
    room: Option<String>,
    saturday: Option<bool>,
    start_date: Option<String>,
    sunday: Option<bool>,
    term: Option<String>,
    thursday: Option<bool>,
    tuesday: Option<bool>,
    wednesday: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Meeting {
    category: Option<String>,
    class: Option<String>,
    course_reference_number: Option<String>,
    faculty: Vec<Faculty>,
    meeting_time: Option<MeetingTime>,
    term: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SectionAttribute {
    class: Option<String>,
    code: Option<String>,
    course_reference_number: Option<String>,
    description: Option<String>,
    is_ztc_attribute: Option<bool>,
    term_code: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Course {
    id: Option<i32>,
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
    maximum_enrollment: Option<i32>,
    enrollment: Option<i32>,
    seats_available: Option<i32>,
    wait_capacity: Option<i32>,
    wait_count: Option<i32>,
    wait_available: Option<i32>,
    cross_list: Option<String>,
    cross_list_capacity: Option<i32>,
    cross_list_count: Option<i32>,
    cross_list_available: Option<i32>,
    credit_hour_high: Option<f32>,
    credit_hour_low: Option<f32>,
    credit_hour_indicator: Option<String>,
    open_section: Option<bool>,
    link_identifier: Option<String>,
    is_section_linked: Option<bool>,
    subject_course: Option<String>,
    faculty: Vec<Faculty>,
    meetings_faculty: Option<Vec<Meeting>>,
    reserved_seat_summary: Option<serde_json::Value>,
    section_attributes: Option<Vec<SectionAttribute>>,
    instructional_method: Option<String>,
    instructional_method_description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ApiResponse {
    success: bool,
    total_count: Option<i32>,
    data: Vec<Course>,
    page_offset: Option<i32>,
    page_max_size: Option<i32>,
    sections_fetched_count: Option<i32>,
    path_mode: Option<String>,
    search_results_configs: Option<serde_json::Value>,
    ztc_encoded_image: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url =
        "https://reg-prod.ec.usfca.edu/StudentRegistrationSsb/ssb/searchResults/searchResults";
    let mut params = HashMap::new();
    params.insert("txt_term", "202510");
    params.insert("startDatepicker", "");
    params.insert("endDatepicker", "");
    params.insert("pageOffset", "0");
    let unique_session_id = std::fs::read_to_string("uniqueSessionId.txt")?
        .trim()
        .to_string();
    params.insert("uniqueSessionId", &unique_session_id);
    params.insert("sortColumn", "subjectDescription");
    params.insert("sortDirection", "asc");

    let mut headers = HeaderMap::new();
    headers.insert("Host", HeaderValue::from_static("reg-prod.ec.usfca.edu"));
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0",
        ),
    );
    headers.insert(
        "Accept",
        HeaderValue::from_static("application/json, text/javascript, */*; q=0.01"),
    );
    let synchronizer_token = std::fs::read_to_string("X-Synchronizer-Token.txt")?
        .trim()
        .to_string();
    headers.insert(
        "X-Synchronizer-Token",
        HeaderValue::from_str(&synchronizer_token)?,
    );
    headers.insert(
        "X-Requested-With",
        HeaderValue::from_static("XMLHttpRequest"),
    );
    headers.insert("DNT", HeaderValue::from_static("1"));
    headers.insert("Sec-GPC", HeaderValue::from_static("1"));
    headers.insert("Connection", HeaderValue::from_static("keep-alive"));
    headers.insert(
        "Referer",
        HeaderValue::from_static(
            "https://reg-prod.ec.usfca.edu/StudentRegistrationSsb/ssb/classSearch/classSearch",
        ),
    );
    let cookie = std::fs::read_to_string("cookie.txt")?.trim().to_string();
    headers.insert("Cookie", HeaderValue::from_str(&cookie)?);
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("same-origin"));
    headers.insert("Priority", HeaderValue::from_static("u=0"));
    headers.insert("TE", HeaderValue::from_static("trailers"));

    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .headers(headers)
        .query(&params)
        .send()
        .await?
        .json::<ApiResponse>()
        .await?;

    let courses: Vec<Course> = response.data;

    let mut wtr = Writer::from_writer(File::create("output.csv")?);

    // Write the header
    wtr.write_record([
        "ID",
        "Term",
        "Term Description",
        "Course Reference Number",
        "Part of Term",
        "Course Number",
        "Subject",
        "Subject Description",
        "Sequence Number",
        "Campus Description",
        "Schedule Type Description",
        "Course Title",
        "Credit Hours",
        "Maximum Enrollment",
        "Enrollment",
        "Seats Available",
        "Wait Capacity",
        "Wait Count",
        "Wait Available",
        "Cross List",
        "Cross List Capacity",
        "Cross List Count",
        "Cross List Available",
        "Credit Hour High",
        "Credit Hour Low",
        "Credit Hour Indicator",
        "Open Section",
        "Link Identifier",
        "Is Section Linked",
        "Subject Course",
        "Instructional Method",
        "Instructional Method Description",
    ])?;

    // Write each course as a row in the CSV
    for course in &courses {
        wtr.write_record(&[
            course.id.map_or("".to_string(), |v| v.to_string()),
            course.term.clone().unwrap_or_default(),
            course.term_desc.clone().unwrap_or_default(),
            course.course_reference_number.clone().unwrap_or_default(),
            course.part_of_term.clone().unwrap_or_default(),
            course.course_number.clone().unwrap_or_default(),
            course.subject.clone().unwrap_or_default(),
            course.subject_description.clone().unwrap_or_default(),
            course.sequence_number.clone().unwrap_or_default(),
            course.campus_description.clone().unwrap_or_default(),
            course.schedule_type_description.clone().unwrap_or_default(),
            course.course_title.clone().unwrap_or_default(),
            course
                .credit_hours
                .map_or("".to_string(), |v| v.to_string()),
            course
                .maximum_enrollment
                .map_or("".to_string(), |v| v.to_string()),
            course.enrollment.map_or("".to_string(), |v| v.to_string()),
            course
                .seats_available
                .map_or("".to_string(), |v| v.to_string()),
            course
                .wait_capacity
                .map_or("".to_string(), |v| v.to_string()),
            course.wait_count.map_or("".to_string(), |v| v.to_string()),
            course
                .wait_available
                .map_or("".to_string(), |v| v.to_string()),
            course.cross_list.clone().unwrap_or_default(),
            course
                .cross_list_capacity
                .map_or("".to_string(), |v| v.to_string()),
            course
                .cross_list_count
                .map_or("".to_string(), |v| v.to_string()),
            course
                .cross_list_available
                .map_or("".to_string(), |v| v.to_string()),
            course
                .credit_hour_high
                .map_or("".to_string(), |v| v.to_string()),
            course
                .credit_hour_low
                .map_or("".to_string(), |v| v.to_string()),
            course.credit_hour_indicator.clone().unwrap_or_default(),
            course
                .open_section
                .map_or("".to_string(), |v| v.to_string()),
            course.link_identifier.clone().unwrap_or_default(),
            course
                .is_section_linked
                .map_or("".to_string(), |v| v.to_string()),
            course.subject_course.clone().unwrap_or_default(),
            course.instructional_method.clone().unwrap_or_default(),
            course
                .instructional_method_description
                .clone()
                .unwrap_or_default(),
        ])?;
    }

    // Flush the writer to ensure all data is written to the file
    wtr.flush()?;
    Ok(())
}
