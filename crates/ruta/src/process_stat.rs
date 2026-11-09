use std::fs;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ProcessStat {
    pub(crate) state: String,
    pub(crate) parent: u32,
    pub(crate) group: u32,
    pub(crate) start: u64,
}

impl ProcessStat {
    // `None` when the process is gone or its stat file cannot be read.
    pub(crate) fn read(pid: u32) -> Option<Self> {
        let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;

        Self::parse(&text)
    }

    // The command name in parentheses may hold spaces and `)`, so the fields start after the last
    // `)`.
    fn parse(text: &str) -> Option<Self> {
        let fields: Vec<&str> = text.rsplit_once(')')?.1.split_whitespace().collect();

        Some(Self {
            state: fields.first()?.to_string(),
            parent: fields.get(1)?.parse().ok()?,
            group: fields.get(2)?.parse().ok()?,
            start: fields.get(19)?.parse().ok()?,
        })
    }

    pub(crate) fn is_zombie(&self) -> bool {
        self.state == "Z"
    }
}

// Now, in the clock of `ProcessStat::start`: clock ticks since boot, counting suspend. A process
// that starts later has a start time at least this large.
pub(crate) fn boot_ticks_now() -> u64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
    let ticks_per_second = rustix::param::clock_ticks_per_second();
    let seconds = u64::try_from(now.tv_sec).expect("the time since boot is not negative");
    let nanoseconds = u64::try_from(now.tv_nsec).expect("nanoseconds are not negative");

    seconds * ticks_per_second + nanoseconds * ticks_per_second / 1_000_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "4242 (a b) c) S 17 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 \
                        987654 1000 200 18446744073709551615";

    #[test]
    fn parse_reads_the_fields_after_a_command_name_with_spaces_and_parens() {
        assert_eq!(
            ProcessStat::parse(STAT),
            Some(ProcessStat {
                state: "S".to_string(),
                parent: 17,
                group: 4242,
                start: 987654,
            })
        );
    }

    #[test]
    fn parse_refuses_a_short_or_broken_line() {
        for (case, text) in [
            ("no parenthesis", "4242 sleep S 17 4242"),
            ("too short", "4242 (sleep) S 17 4242"),
            ("bad parent", STAT.replace(" 17 ", " x ").as_str()),
            ("empty", ""),
        ] {
            assert_eq!(ProcessStat::parse(text), None, "{case}");
        }
    }

    #[test]
    fn is_zombie_reads_the_state() {
        let zombie = ProcessStat::parse(&STAT.replace(") S ", ") Z ")).expect("a zombie line");
        let sleeping = ProcessStat::parse(STAT).expect("a sleeping line");

        assert!(zombie.is_zombie());
        assert!(!sleeping.is_zombie());
    }

    #[test]
    fn boot_ticks_now_uses_the_clock_of_process_start_times() {
        let before = boot_ticks_now();
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("start a process");
        let started = ProcessStat::read(child.id()).map(|stat| stat.start);
        child.wait().expect("reap the process");
        let after = boot_ticks_now();

        let started = started.expect("the process has a start time");
        assert!(
            before <= started && started <= after,
            "{before} <= {started} <= {after}"
        );
    }

    #[test]
    fn read_names_the_test_process_and_nothing_for_a_missing_pid() {
        let stat = ProcessStat::read(std::process::id()).expect("the test process has a stat");

        assert!(!stat.is_zombie());
        assert_eq!(ProcessStat::read(u32::MAX), None);
    }
}
