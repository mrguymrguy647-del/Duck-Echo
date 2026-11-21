//! How long Duck Echo waits for each kind of request, in one table.
//!
//! Duck Echo never waits for the Ear longer than this. When the time is
//! up, the Ear is stopped (it may be stuck) and the next question starts
//! it again. See `docs/phase4-design.md`, section 3, "Timeouts".

use serde_json::Value;
use std::time::Duration;

/// The environment variable that makes every time longer (or shorter), for
/// a slow PC: `DUCK_ECHO_EAR_TIMEOUT_SCALE=2` doubles them all.
pub const SCALE_VARIABLE: &str = "DUCK_ECHO_EAR_TIMEOUT_SCALE";

/// The smallest and largest scale allowed. No scale makes a wait endless.
pub const SCALE_RANGE: (f64, f64) = (0.25, 10.0);

/// The time allowed for each kind of request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Times {
    /// Starting the Ear and its answer to `hello`. Python's first start on
    /// Windows, with an antivirus scanning it, can take 10 seconds.
    pub hello: Duration,
    /// `load` of the score model (under 1 GB, from disk).
    pub load_score: Duration,
    /// `load` of the describe model (several GB), which also starts
    /// `llama-server` again if it was stopped.
    pub load_describe: Duration,
    /// `score`: one sound, about a second on a CPU.
    pub score: Duration,
    /// `describe` and `ask` when the model runs on a graphics card.
    pub describe_on_card: Duration,
    /// `describe` and `ask` when the model runs on the CPU.
    pub describe_on_cpu: Duration,
    /// `bye`: then the Ear is stopped anyway.
    pub bye: Duration,
}

impl Default for Times {
    fn default() -> Times {
        let seconds = Duration::from_secs;
        Times {
            hello: seconds(20),
            load_score: seconds(60),
            load_describe: seconds(180),
            score: seconds(15),
            describe_on_card: seconds(60),
            describe_on_cpu: seconds(240),
            bye: seconds(3),
        }
    }
}

impl Times {
    /// Every time multiplied by `by`, kept inside [`SCALE_RANGE`].
    pub fn scaled(self, by: f64) -> Times {
        let by = by.clamp(SCALE_RANGE.0, SCALE_RANGE.1);
        let scale = |time: Duration| time.mul_f64(by);
        Times {
            hello: scale(self.hello),
            load_score: scale(self.load_score),
            load_describe: scale(self.load_describe),
            score: scale(self.score),
            describe_on_card: scale(self.describe_on_card),
            describe_on_cpu: scale(self.describe_on_cpu),
            bye: scale(self.bye),
        }
    }

    /// The usual times, scaled by [`SCALE_VARIABLE`] when it is set. A value
    /// that is not a number is an error that says so.
    pub fn from_environment() -> Result<Times, String> {
        Times::from_scale_text(std::env::var(SCALE_VARIABLE).ok().as_deref())
    }

    /// The usual times, scaled by `text` (the value of [`SCALE_VARIABLE`]),
    /// or unscaled when there is none.
    pub fn from_scale_text(text: Option<&str>) -> Result<Times, String> {
        match text {
            None => Ok(Times::default()),
            Some(text) => match text.trim().parse::<f64>() {
                Ok(by) if by.is_finite() && by > 0.0 => Ok(Times::default().scaled(by)),
                _ => Err(format!(
                    "{SCALE_VARIABLE} is `{text}`: it must be a number from {} to {}, such as 2",
                    SCALE_RANGE.0, SCALE_RANGE.1
                )),
            },
        }
    }

    /// The time allowed for `method` with these `params`, on an Ear whose
    /// models run on `device` (`cpu`, `vulkan` ...). A method this table
    /// does not know gets the shortest wait, `score`'s.
    pub fn for_request(&self, method: &str, params: &Value, device: &str) -> Duration {
        match method {
            "hello" => self.hello,
            "load" => {
                let wants_describe = params["roles"]
                    .as_array()
                    .is_some_and(|roles| roles.iter().any(|role| role == "describe"));
                if wants_describe {
                    self.load_describe
                } else {
                    self.load_score
                }
            }
            "describe" | "ask" if device == "cpu" => self.describe_on_cpu,
            "describe" | "ask" => self.describe_on_card,
            "bye" => self.bye,
            _ => self.score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_request_gets_its_own_time() {
        let times = Times::default();
        let none = json!({});
        assert_eq!(
            times.for_request("hello", &none, "cpu"),
            Duration::from_secs(20)
        );
        assert_eq!(
            times.for_request("load", &json!({"roles": ["score"]}), "cpu"),
            Duration::from_secs(60)
        );
        assert_eq!(
            times.for_request("load", &json!({"roles": ["score", "describe"]}), "cpu"),
            Duration::from_secs(180)
        );
        assert_eq!(
            times.for_request("score", &none, "cpu"),
            Duration::from_secs(15)
        );
        assert_eq!(
            times.for_request("ask", &none, "vulkan"),
            Duration::from_secs(60)
        );
        assert_eq!(
            times.for_request("ask", &none, "cpu"),
            Duration::from_secs(240)
        );
        assert_eq!(
            times.for_request("describe", &none, "cpu"),
            Duration::from_secs(240)
        );
        assert_eq!(
            times.for_request("bye", &none, "cpu"),
            Duration::from_secs(3)
        );
        assert_eq!(
            times.for_request("something_new", &none, "cpu"),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn the_scale_comes_from_its_variable_and_must_be_a_number() {
        assert_eq!(Times::from_scale_text(None), Ok(Times::default()));
        assert_eq!(
            Times::from_scale_text(Some(" 2 ")).unwrap().hello,
            Duration::from_secs(40)
        );
        for bad in ["", "two", "-1", "0", "NaN", "inf"] {
            let error = Times::from_scale_text(Some(bad)).unwrap_err();
            assert!(error.contains("must be a number"), "{bad}: {error}");
        }
    }

    #[test]
    fn the_scale_is_kept_inside_its_range() {
        let times = Times::default();
        assert_eq!(times.scaled(2.0).score, Duration::from_secs(30));
        assert_eq!(times.scaled(1000.0).bye, Duration::from_secs(30));
        assert_eq!(times.scaled(0.0).score, Duration::from_secs_f64(3.75));
    }
}
