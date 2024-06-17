use std::time::{SystemTime, UNIX_EPOCH, Instant};

/// Get the current timestamp in seconds since the UNIX epoch
pub fn current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// Check if the provided timestamp is recent (within the last 10 seconds)
pub fn is_recent(timestamp: u64) -> bool {
    let now = current_timestamp();
    now <= timestamp + 10 // Accept messages that are at most 10 seconds old
}

/// Log a message with a timestamp based on the start time of the program
pub fn log_with_timestamp(start_time: Instant, message: &str) {
    let elapsed = start_time.elapsed();
    let seconds = elapsed.as_secs();
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let formatted_time = format!("{:02}:{:02}:{:02}", hours % 24, minutes % 60, seconds % 60);
    println!("{} - {}", formatted_time, message);
}
