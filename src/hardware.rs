use hwlocality::{
    cpu::{cpuset::CpuSet, kind::CpuKind},
    ffi::PositiveInt,
    object::types::ObjectType,
    topology::builder::TypeFilter,
    Topology,
};

/// A single CPU core.
pub struct Core<'a> {
    inner: CpuKind<'a>,
    topology: &'a Topology,
}

impl Core<'_> {
    /// Numerical ID of the CPU core.
    pub fn id(&self) -> PositiveInt {
        self.inner.cpuset.iter_set().next().unwrap()
    }

    /// CPU set that corresponds to only this core.
    pub fn cpu_set(&self) -> CpuSet {
        self.inner.cpuset.clone()
    }

    /// Model name of the CPU core, `None` if unknown.
    pub fn model(&self) -> Option<String> {
        // We for now only implement this on ARM. On all other architectures,
        // it should be possible to get a reasonable name from the package
        // instead.

        let package = self
            .topology
            .objects_with_type(ObjectType::Package)
            .next()
            .unwrap();

        // Common attributes are on the package, specific ones on the CPU core
        let get_cpu_attr = |key| {
            self.inner
                .infos
                .iter()
                .find_map(|i| {
                    if i.name().to_str() == Ok(key) {
                        Some(i.value())
                    } else {
                        None
                    }
                })
                .or_else(|| package.info(key))
                .map(|i| i.to_str().unwrap())
        };

        let implementer_raw =
            u8::from_str_radix(get_cpu_attr("CPUImplementer")?.trim_start_matches("0x"), 16)
                .unwrap();
        let part_raw =
            u16::from_str_radix(get_cpu_attr("CPUPart")?.trim_start_matches("0x"), 16).unwrap();
        let variant_raw =
            u8::from_str_radix(get_cpu_attr("CPUVariant")?.trim_start_matches("0x"), 16).unwrap();

        let implementer = libcpuname::arm::Implementer::try_from(implementer_raw).ok()?;
        let core = libcpuname::arm::core_name(implementer, part_raw, variant_raw).ok()?;

        Some(format!("{implementer} {core}"))
    }
}

pub struct HardwareInformation(Topology);

impl HardwareInformation {
    /// Gather hardware information from the running system.
    pub fn gather() -> Self {
        Self(
            Topology::builder()
                .with_common_type_filter(TypeFilter::KeepAll)
                .unwrap()
                .build()
                .unwrap(),
        )
    }

    /// Pretty name of the machine. This is the name from DMI data if available, otherwise the hostname.
    pub fn machine_name(&self) -> &str {
        let machine = self.0.root_object();
        // Not all machines provide DMI information, but HostName should always be set
        machine
            .info("DMIProductName")
            .or(machine.info("HostName"))
            .unwrap()
            .to_str()
            .unwrap()
            .trim()
    }

    /// Hostname of the machine.
    pub fn hostname(&self) -> &str {
        let machine = self.0.root_object();
        machine.info("HostName").unwrap().to_str().unwrap()
    }

    /// Architecture of the machine. The values here are the ones also used by GitHub Actions' `RUNNER_ARCH` variable.
    ///
    /// See <https://docs.github.com/en/actions/reference/workflows-and-actions/variables#default-environment-variables>
    pub fn arch(&self) -> &str {
        let machine = self.0.root_object();
        let arch = machine
            .info("Architecture")
            .unwrap()
            .to_str()
            .unwrap()
            .trim();

        // Normalize the value to the GitHub reported values
        match arch {
            "i686" => "X86",
            "x86_64" => "X64",
            "armv7l" | "armv8l" => "ARM",
            "aarch64" | "arm64" => "ARM64",
            // Other platforms are not supported officially, but can e.g. use
            // the act-based runner here:
            // https://github.com/ChristopherHX/github-act-runner
            // This runner reports `RUNNER_ARCH` as the `GOARCH` value
            // We will presume that `GOARCH` == `uname -m` and list exceptions
            // here explicitly
            "loongarch64" => "loong64",
            _ => arch,
        }
    }

    /// Operating system of the machine. The values here are the ones also used by GitHub Actions' `RUNNER_OS` variable.
    ///
    /// See <https://docs.github.com/en/actions/reference/workflows-and-actions/variables#default-environment-variables>
    pub fn os(&self) -> &str {
        let machine = self.0.root_object();
        let os_name = machine.info("OSName").unwrap().to_str().unwrap().trim();

        // The odd one out here is macOS, which gets reported as Darwin
        // TODO: Check Windows
        match os_name {
            "Darwin" => "macOS",
            _ => os_name,
        }
    }

    /// Name of the CPU model. `None` if unknown.
    pub fn cpu_model(&self) -> Option<&str> {
        let package = self
            .0
            .objects_with_type(ObjectType::Package)
            .next()
            .unwrap();

        package
            .info("CPUModel")
            .map(|model| model.to_str().unwrap().trim())
    }

    /// Number of a single CPU core best suited for running a benchmark on.
    ///
    /// This is a core on the most performant cluster which, if possible, is not core 0.
    pub fn best_benchmark_cpu(&self) -> Core<'_> {
        // cpu_kinds() are sorted ascending by performance, so we pick a core from the last group
        let mut best_cpu_kind = self.0.cpu_kinds().unwrap().last().unwrap();
        // We don't want to run on core 0 if we can avoid it, it usually handles IRQs
        if best_cpu_kind.cpuset.weight().is_none_or(|w| w >= 1) {
            best_cpu_kind.cpuset.unset(0);
        }
        // Reduce the set to a single PU
        best_cpu_kind.cpuset.singlify();

        Core {
            inner: best_cpu_kind,
            topology: &self.0,
        }
    }
}
