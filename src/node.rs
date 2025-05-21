#[cfg(test)]
mod tests {
    use bitcoind::{BitcoinD, Conf};
    use std::env;
    use std::ffi::OsStr;

    fn launch_elementsd<S: AsRef<OsStr>>(exe: S) -> BitcoinD {
        let mut conf = Conf::default();
        let args = vec![
            "-fallbackfee=0.0001",
            "-dustrelayfee=0.00000001",
            "-chain=liquidregtest",
            "-initialfreecoins=2100000000",
            "-validatepegin=0",
            "-acceptdiscountct=1",
            "-txindex=1",
            "-rest=1",
        ];
        conf.args = args;
        conf.view_stdout = std::env::var("RUST_LOG").is_ok();
        conf.network = "liquidregtest";

        BitcoinD::with_conf(exe, &conf).unwrap()
    }

    #[test]
    fn test_launch_elementsd() {
        let elementsd_exe = env::var("ELEMENTSD_EXEC").expect("ELEMENTSD_EXEC must be set");
        let _elementsd = launch_elementsd(elementsd_exe);
    }
}
