use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;

use crate::error::{ErrorCode, TimerError, TimerResult};

fn parse_int(v: &str, min: u32, max: u32, label: &str) -> TimerResult<u32> {
  let n = v.parse::<u32>().map_err(|_| {
    TimerError::new(
      ErrorCode::InvalidCronExpression,
      format!("{label} field is not a valid number: {v}"),
    )
  })?;
  if n < min || n > max {
    return Err(TimerError::new(
      ErrorCode::InvalidCronExpression,
      format!("{label} field out of range {min}..{max}: {n}"),
    ));
  }
  Ok(n)
}

fn validate_token(token: &str, min: u32, max: u32, label: &str) -> TimerResult<()> {
  if token == "*" {
    return Ok(());
  }

  if let Some((base, step)) = token.split_once('/') {
    let step_num = parse_int(step, 1, max, label)?;
    if step_num == 0 {
      return Err(TimerError::new(
        ErrorCode::InvalidCronExpression,
        format!("{label} step cannot be zero"),
      ));
    }
    if base == "*" {
      return Ok(());
    }
    if let Some((start, end)) = base.split_once('-') {
      let start_num = parse_int(start, min, max, label)?;
      let end_num = parse_int(end, min, max, label)?;
      if start_num > end_num {
        return Err(TimerError::new(
          ErrorCode::InvalidCronExpression,
          format!("{label} range start > end in {token}"),
        ));
      }
      return Ok(());
    }
    return Err(TimerError::new(
      ErrorCode::InvalidCronExpression,
      format!("{label} invalid step expression: {token}"),
    ));
  }

  if let Some((start, end)) = token.split_once('-') {
    let start_num = parse_int(start, min, max, label)?;
    let end_num = parse_int(end, min, max, label)?;
    if start_num > end_num {
      return Err(TimerError::new(
        ErrorCode::InvalidCronExpression,
        format!("{label} range start > end in {token}"),
      ));
    }
    return Ok(());
  }

  parse_int(token, min, max, label)?;
  Ok(())
}

fn validate_field(raw: &str, min: u32, max: u32, label: &str) -> TimerResult<()> {
  for token in raw.split(',') {
    let token = token.trim();
    if token.is_empty() {
      return Err(TimerError::new(
        ErrorCode::InvalidCronExpression,
        format!("{label} contains an empty token"),
      ));
    }
    validate_token(token, min, max, label)?;
  }
  Ok(())
}

pub fn normalize_and_validate_cron(expr: &str) -> TimerResult<String> {
  if expr.contains('L') || expr.contains('W') || expr.contains('#') {
    return Err(TimerError::new(
      ErrorCode::UnsupportedCronSyntax,
      "cron syntax L/W/# is not supported",
    ));
  }

  let fields = expr.split_whitespace().collect::<Vec<_>>();
  let normalized_fields = match fields.len() {
    5 => vec!["0", fields[0], fields[1], fields[2], fields[3], fields[4]],
    6 => fields,
    _ => {
      return Err(TimerError::new(
        ErrorCode::InvalidCronExpression,
        "cron must have 5 fields (min hour dom mon dow) or 6 fields (sec min hour dom mon dow)",
      ));
    }
  };

  validate_field(normalized_fields[0], 0, 59, "second")?;
  validate_field(normalized_fields[1], 0, 59, "minute")?;
  validate_field(normalized_fields[2], 0, 23, "hour")?;
  validate_field(normalized_fields[3], 1, 31, "day_of_month")?;
  validate_field(normalized_fields[4], 1, 12, "month")?;
  validate_field(normalized_fields[5], 0, 7, "day_of_week")?;

  Ok(normalized_fields.join(" "))
}

pub fn next_cron_due(
  expr: &str,
  tz_name: &str,
  after: DateTime<Utc>,
) -> TimerResult<DateTime<Utc>> {
  let normalized = normalize_and_validate_cron(expr)?;
  let tz = Tz::from_str(tz_name).map_err(|_| {
    TimerError::new(
      ErrorCode::InvalidArgument,
      format!("invalid timezone: {tz_name}"),
    )
  })?;

  let schedule = Schedule::from_str(&normalized).map_err(|e| {
    TimerError::new(
      ErrorCode::InvalidCronExpression,
      format!("unable to parse cron expression: {e}"),
    )
  })?;

  let after_local = tz.from_utc_datetime(&after.naive_utc());
  let next_local = schedule.after(&after_local).next().ok_or_else(|| {
    TimerError::new(
      ErrorCode::InvalidCronExpression,
      "cron has no future execution time",
    )
  })?;
  Ok(next_local.with_timezone(&Utc))
}

pub fn every_minutes(n: u32) -> TimerResult<String> {
  parse_int(&n.to_string(), 1, 59, "minute_interval")?;
  Ok(format!("*/{n} * * * *"))
}

pub fn every_hours(n: u32) -> TimerResult<String> {
  parse_int(&n.to_string(), 1, 23, "hour_interval")?;
  Ok(format!("0 */{n} * * *"))
}

pub fn daily_at(hour: u32, minute: u32) -> TimerResult<String> {
  parse_int(&hour.to_string(), 0, 23, "hour")?;
  parse_int(&minute.to_string(), 0, 59, "minute")?;
  Ok(format!("{minute} {hour} * * *"))
}

pub fn weekly_on(dow: u32, hour: u32, minute: u32) -> TimerResult<String> {
  parse_int(&dow.to_string(), 0, 7, "day_of_week")?;
  parse_int(&hour.to_string(), 0, 23, "hour")?;
  parse_int(&minute.to_string(), 0, 59, "minute")?;
  Ok(format!("{minute} {hour} * * {dow}"))
}

pub fn monthly_on(day: u32, hour: u32, minute: u32) -> TimerResult<String> {
  parse_int(&day.to_string(), 1, 31, "day_of_month")?;
  parse_int(&hour.to_string(), 0, 23, "hour")?;
  parse_int(&minute.to_string(), 0, 59, "minute")?;
  Ok(format!("{minute} {hour} {day} * *"))
}

#[cfg(test)]
mod tests {
  use chrono::{TimeZone, Utc};

  use super::{daily_at, next_cron_due, normalize_and_validate_cron};

  #[test]
  fn test_normalize_cron() {
    let got = normalize_and_validate_cron("30 3 * * *").expect("normalize ok");
    assert_eq!(got, "0 30 3 * * *");
  }

  #[test]
  fn test_daily_at() {
    let got = daily_at(3, 30).expect("daily_at ok");
    assert_eq!(got, "30 3 * * *");
  }

  #[test]
  fn test_next_due_dst_safe() {
    let start = Utc.with_ymd_and_hms(2026, 3, 8, 6, 50, 0).unwrap();
    let next = next_cron_due("30 3 * * *", "America/New_York", start).expect("next due");
    assert!(next > start);
  }
}
