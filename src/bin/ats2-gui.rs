use std::{
    collections::BTreeMap,
    env, fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

use ats2_rs::sweep::{
    SweepAxisSpec, SweepProgress,
    demo::{
        ControlledDemonstrationResult, DemoConfig, DemoSweepVariable, DemoXAxis,
        DemonstrationResult, SIMULATION_NOTICE, simulate_demo_controlled,
    },
};
use eframe::egui;
use egui_plot::{Legend, Line, Plot};

fn main() -> eframe::Result<()> {
    if !env::args().any(|argument| argument == "--mock") {
        eprintln!(
            "This GUI is mock-only. Start it explicitly with --mock; no ATS-2 backend is available."
        );
        std::process::exit(2);
    }

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "ATS-2 — Mock GUI",
        options,
        Box::new(|_creation_context| Ok(Box::new(GuiApp::default()))),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceChoice {
    Mock,
    Ats2Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SweepSpacing {
    Fixture,
    Linear,
    Logarithmic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XMetric {
    SimulatedLevel,
    FrequencyHz,
    MeasuredVrms,
    MeasuredPowerW,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YMetric {
    MeasuredVrms,
    ThdN,
    Thd,
}

impl YMetric {
    const fn label(self) -> &'static str {
        match self {
            Self::MeasuredVrms => "RMS voltage",
            Self::ThdN => "THD+N",
            Self::Thd => "THD",
        }
    }

    const fn unit(self) -> &'static str {
        match self {
            Self::MeasuredVrms => "V RMS",
            Self::ThdN | Self::Thd => "%",
        }
    }

    fn value(
        self,
        record: &ats2_rs::sweep::MultiSweepRecord<f64, ats2_rs::sweep::demo::DemoError>,
    ) -> Option<f64> {
        match self {
            Self::MeasuredVrms => measurement_value(record, "voltage.rms"),
            Self::ThdN => measurement_value(record, "distortion.thd+n"),
            Self::Thd => measurement_value(record, "distortion.thd"),
        }
    }
}

enum WorkerMessage {
    Progress(SweepProgress),
    Finished(Result<Box<ControlledDemonstrationResult>, String>),
}

struct Worker {
    events: Receiver<WorkerMessage>,
    cancel: Arc<AtomicBool>,
}

struct GuiApp {
    device: DeviceChoice,
    spacing: SweepSpacing,
    sweep_variable: DemoSweepVariable,
    start_level_vrms: f64,
    end_level_vrms: f64,
    start_frequency_hz: f64,
    end_frequency_hz: f64,
    point_count: usize,
    include_thd: bool,
    calculate_power: bool,
    load_resistance_ohms: f64,
    x_metric: XMetric,
    show_rms: bool,
    show_thd_n: bool,
    show_thd: bool,
    log_x: bool,
    log_y: bool,
    selected_point: Option<usize>,
    output_path: String,
    worker: Option<Worker>,
    progress: Option<SweepProgress>,
    result: Option<ControlledDemonstrationResult>,
    status: String,
    error: Option<String>,
}

impl Default for GuiApp {
    fn default() -> Self {
        Self {
            device: DeviceChoice::Mock,
            spacing: SweepSpacing::Fixture,
            sweep_variable: DemoSweepVariable::GeneratorLevelVrms,
            start_level_vrms: 0.1,
            end_level_vrms: 1.0,
            start_frequency_hz: 20.0,
            end_frequency_hz: 20_000.0,
            point_count: 5,
            include_thd: true,
            calculate_power: false,
            load_resistance_ohms: 8.0,
            x_metric: XMetric::SimulatedLevel,
            show_rms: true,
            show_thd_n: true,
            show_thd: false,
            log_x: false,
            log_y: false,
            selected_point: None,
            output_path: "ats2-sweep".to_owned(),
            worker: None,
            progress: None,
            result: None,
            status: "Bereit — Mock-Modus aktiv.".to_owned(),
            error: None,
        }
    }
}

impl GuiApp {
    fn demo_config(&self) -> DemoConfig {
        let axis = match (self.sweep_variable, self.spacing) {
            (DemoSweepVariable::GeneratorLevelVrms, SweepSpacing::Fixture) => {
                SweepAxisSpec::Explicit {
                    values: vec![0.1, 0.5, 1.0],
                }
            }
            (DemoSweepVariable::GeneratorLevelVrms, SweepSpacing::Linear) => {
                SweepAxisSpec::Linear {
                    start: self.start_level_vrms,
                    end: self.end_level_vrms,
                    points: self.point_count,
                }
            }
            (DemoSweepVariable::GeneratorLevelVrms, SweepSpacing::Logarithmic) => {
                SweepAxisSpec::Logarithmic {
                    start: self.start_level_vrms,
                    end: self.end_level_vrms,
                    points: self.point_count,
                }
            }
            (DemoSweepVariable::FrequencyHz, SweepSpacing::Fixture) => SweepAxisSpec::Logarithmic {
                start: self.start_frequency_hz,
                end: self.end_frequency_hz,
                points: self.point_count,
            },
            (DemoSweepVariable::FrequencyHz, SweepSpacing::Linear) => SweepAxisSpec::Linear {
                start: self.start_frequency_hz,
                end: self.end_frequency_hz,
                points: self.point_count,
            },
            (DemoSweepVariable::FrequencyHz, SweepSpacing::Logarithmic) => {
                SweepAxisSpec::Logarithmic {
                    start: self.start_frequency_hz,
                    end: self.end_frequency_hz,
                    points: self.point_count,
                }
            }
        };
        DemoConfig {
            display_axis: match self.x_metric {
                XMetric::SimulatedLevel => DemoXAxis::GeneratorLevelVrms,
                XMetric::FrequencyHz => DemoXAxis::FrequencyHz,
                XMetric::MeasuredVrms => DemoXAxis::MeasuredVrms,
                XMetric::MeasuredPowerW => DemoXAxis::MeasuredPowerW,
            },
            sweep_variable: self.sweep_variable,
            axis,
            include_thd: self.include_thd,
            load_resistance_ohms: self.calculate_power.then_some(self.load_resistance_ohms),
        }
    }

    fn start_sweep(&mut self) {
        if self.device != DeviceChoice::Mock {
            self.error = Some("ATS-2-Hardwarebetrieb ist noch nicht implementiert.".to_owned());
            return;
        }
        if self.point_count < 2 || self.point_count > 10_000 {
            self.error = Some("Punktzahl muss zwischen 2 und 10000 liegen.".to_owned());
            return;
        }
        let config = self.demo_config();
        if let Err(error) = config
            .axis
            .generate_values(ats2_rs::sweep::SweepOrder::AsSpecified)
        {
            self.error = Some(error.to_string());
            return;
        }
        let total_points = config_point_count(&self.spacing, self.point_count);

        let (sender, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        thread::spawn(move || {
            let result = simulate_demo_controlled(
                &config,
                || worker_cancel.load(Ordering::Relaxed),
                |progress| {
                    let _ = sender.send(WorkerMessage::Progress(progress));
                },
            )
            .map(Box::new)
            .map_err(|error| error.to_string());
            let _ = sender.send(WorkerMessage::Finished(result));
        });

        self.worker = Some(Worker { events, cancel });
        self.progress = Some(SweepProgress {
            completed_points: 0,
            total_points,
        });
        self.selected_point = None;
        self.error = None;
        self.status = "Mock-Sweep läuft im Worker-Thread.".to_owned();
    }

    fn receive_worker_events(&mut self) {
        let messages: Vec<_> = self
            .worker
            .as_ref()
            .map(|worker| worker.events.try_iter().collect())
            .unwrap_or_default();
        let mut finished = None;
        for message in messages {
            match message {
                WorkerMessage::Progress(progress) => self.progress = Some(progress),
                WorkerMessage::Finished(result) => finished = Some(result),
            }
        }
        if let Some(result) = finished {
            self.worker = None;
            match result {
                Ok(run) => {
                    self.status = if run.cancelled_at.is_some() {
                        "Abgebrochen — abgeschlossene Punkte bleiben verfügbar.".to_owned()
                    } else if let Some(index) = run.demonstration.sweep.aborted_at {
                        format!("Messfehler an Punkt {index}; Fehlerdetails bleiben exportierbar.")
                    } else {
                        "Mock-Sweep abgeschlossen.".to_owned()
                    };
                    self.result = Some(*run);
                }
                Err(error) => {
                    self.error = Some(error.clone());
                    self.status = "Mock-Sweep fehlgeschlagen.".to_owned();
                }
            }
        }
    }

    fn request_cancel(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel.store(true, Ordering::Relaxed);
            self.status = "Abbruch angefordert; wirksam zwischen Messpunkten.".to_owned();
        }
    }

    fn export(&mut self, json: bool) {
        let Some(run) = &self.result else {
            self.error = Some("Es gibt noch kein Sweep-Ergebnis zum Exportieren.".to_owned());
            return;
        };
        let mut metadata = BTreeMap::new();
        metadata.insert("source".to_owned(), "mock simulation".to_owned());
        metadata.insert("provenance".to_owned(), SIMULATION_NOTICE.to_owned());
        if let Some(cancelled_at) = run.cancelled_at {
            metadata.insert("cancelled_at".to_owned(), cancelled_at.to_string());
        }
        let extension = if json { "json" } else { "csv" };
        let path = format!("{}.{}", self.output_path.trim(), extension);
        let contents = if json {
            run.export_json(&metadata)
        } else {
            run.export_csv(&metadata)
        };
        match contents {
            Ok(contents) => match fs::write(&path, contents) {
                Ok(()) => {
                    self.error = None;
                    self.status = format!("Export geschrieben: {path}");
                }
                Err(error) => self.error = Some(format!("Export fehlgeschlagen: {error}")),
            },
            Err(error) => self.error = Some(format!("Export fehlgeschlagen: {error}")),
        }
    }

    fn show_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Gerät und Generator");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.device, DeviceChoice::Mock, "Mock / Simulation");
            ui.add_enabled_ui(false, |ui| {
                ui.radio_value(
                    &mut self.device,
                    DeviceChoice::Ats2Unavailable,
                    "ATS-2 (nicht implementiert)",
                );
            });
        });
        ui.label("Der Generator-Ausgang ist ausschließlich ein synthetischer Sweep-Stimulus.");

        ui.separator();
        ui.heading("Analyzer-Einstellungen");
        ui.label("Kanal A (simuliert); RMS und THD+N sind enthalten.");
        ui.checkbox(&mut self.include_thd, "Zusätzliche THD-Kurve simulieren");
        ui.checkbox(
            &mut self.calculate_power,
            "Leistung für expliziten Lastwiderstand berechnen",
        );
        ui.add_enabled_ui(self.calculate_power, |ui| {
            ui.horizontal(|ui| {
                ui.label("R (Ohm):");
                ui.add(egui::DragValue::new(&mut self.load_resistance_ohms).speed(0.1));
            });
        });

        ui.separator();
        ui.heading("Sweep-Konfiguration");
        ui.horizontal(|ui| {
            ui.label("Sweep-Variable:");
            ui.radio_value(
                &mut self.sweep_variable,
                DemoSweepVariable::GeneratorLevelVrms,
                "Ausgangspegel",
            );
            ui.radio_value(
                &mut self.sweep_variable,
                DemoSweepVariable::FrequencyHz,
                "Frequenz",
            );
        });
        if self.sweep_variable == DemoSweepVariable::FrequencyHz
            && self.spacing == SweepSpacing::Fixture
        {
            self.spacing = SweepSpacing::Logarithmic;
        }
        ui.label(match self.sweep_variable {
            DemoSweepVariable::GeneratorLevelVrms => {
                "Mock sweep: synthetischer Ausgangspegel bei fest 1 kHz."
            }
            DemoSweepVariable::FrequencyHz => {
                "Mock sweep: synthetische Frequenz bei fest 0.5 V RMS."
            }
        });
        ui.horizontal(|ui| {
            ui.add_enabled_ui(
                self.sweep_variable == DemoSweepVariable::GeneratorLevelVrms,
                |ui| {
                    ui.radio_value(
                        &mut self.spacing,
                        SweepSpacing::Fixture,
                        "Feste Mock-Fixture",
                    );
                },
            );
            ui.radio_value(&mut self.spacing, SweepSpacing::Linear, "Linear");
            ui.radio_value(
                &mut self.spacing,
                SweepSpacing::Logarithmic,
                "Logarithmisch",
            );
        });
        let configurable = self.spacing != SweepSpacing::Fixture;
        ui.add_enabled_ui(configurable, |ui| {
            ui.horizontal(|ui| {
                match self.sweep_variable {
                    DemoSweepVariable::GeneratorLevelVrms => {
                        ui.label("Start (V RMS):");
                        ui.add(egui::DragValue::new(&mut self.start_level_vrms).speed(0.01));
                        ui.label("Ende (V RMS):");
                        ui.add(egui::DragValue::new(&mut self.end_level_vrms).speed(0.01));
                    }
                    DemoSweepVariable::FrequencyHz => {
                        ui.label("Start (Hz):");
                        ui.add(egui::DragValue::new(&mut self.start_frequency_hz).speed(1.0));
                        ui.label("Ende (Hz):");
                        ui.add(egui::DragValue::new(&mut self.end_frequency_hz).speed(10.0));
                    }
                }
                ui.label("Punkte:");
                ui.add(egui::DragValue::new(&mut self.point_count).speed(1));
            });
        });
        if self.spacing == SweepSpacing::Fixture
            && self.sweep_variable == DemoSweepVariable::GeneratorLevelVrms
        {
            ui.label("Fixture-Stimuli: 0.1, 0.5 und 1.0 V RMS — ausschließlich synthetische Beispieldaten.");
        }
        if self.x_metric == XMetric::MeasuredPowerW && !self.calculate_power {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Für die Leistungsachse muss die Berechnung mit explizitem Lastwiderstand aktiviert sein.",
            );
        }

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.worker.is_none()
                        && (self.x_metric != XMetric::MeasuredPowerW || self.calculate_power),
                    egui::Button::new("Sweep starten (Mock)"),
                )
                .clicked()
            {
                self.start_sweep();
            }
            if ui
                .add_enabled(self.worker.is_some(), egui::Button::new("Abbrechen"))
                .clicked()
            {
                self.request_cancel();
            }
        });
        if let Some(progress) = self.progress {
            ui.label(format!(
                "Fortschritt: {} / {} Punkte",
                progress.completed_points, progress.total_points
            ));
        }
    }

    fn show_results(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Gespeicherte Ergebnisse");
        ui.horizontal(|ui| {
            ui.label("X-Achse für nächsten und aktuellen Sweep:");
            egui::ComboBox::from_id_salt("x_metric")
                .selected_text(match self.x_metric {
                    XMetric::SimulatedLevel => "Simulierter Generator-Level",
                    XMetric::FrequencyHz => "Frequenz (Hz)",
                    XMetric::MeasuredVrms => "Gemessener RMS-Wert",
                    XMetric::MeasuredPowerW => "Gemessene Leistung (W, Mock-Last)",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.x_metric, XMetric::FrequencyHz, "Frequenz (Hz)");
                    ui.selectable_value(
                        &mut self.x_metric,
                        XMetric::SimulatedLevel,
                        "Simulierter Generator-Level",
                    );
                    ui.selectable_value(
                        &mut self.x_metric,
                        XMetric::MeasuredVrms,
                        "Gemessener RMS-Wert",
                    );
                    ui.add_enabled_ui(self.calculate_power, |ui| {
                        ui.selectable_value(
                            &mut self.x_metric,
                            XMetric::MeasuredPowerW,
                            "Gemessene Leistung (W, Mock-Last)",
                        );
                    });
                });
        });
        ui.horizontal(|ui| {
            ui.label("Skalen:");
            ui.checkbox(&mut self.log_x, "X logarithmisch");
            ui.checkbox(&mut self.log_y, "Y logarithmisch");
            ui.separator();
            ui.label("Kurven:");
            ui.checkbox(&mut self.show_rms, "RMS");
            ui.checkbox(&mut self.show_thd_n, "THD+N");
            ui.checkbox(&mut self.show_thd, "THD");
        });
        let Some(run) = &self.result else {
            ui.label("Noch kein Ergebnis. Ein Mock-Sweep erzeugt keine Hardwarezugriffe.");
            return;
        };
        ui.label(run.demonstration.provenance);
        if let Some(cancelled_at) = run.cancelled_at {
            ui.label(format!("Abbruch vor Punkt {cancelled_at}; nicht ausgeführte Punkte bleiben im Export markiert."));
        }
        if let Some(assumptions) = run.demonstration.power_assumptions {
            ui.label(assumptions);
        }
        if let Some(resistance) = run.demonstration.load_resistance_ohms {
            ui.label(format!("Explizite Mock-Last: {resistance} Ω"));
        }

        egui::ScrollArea::horizontal().show(ui, |ui| {
            egui::Grid::new("result_table")
                .striped(true)
                .show(ui, |ui| {
                    for heading in [
                        "Punkt",
                        "Sweep-Achse",
                        "Stimulus (V RMS)",
                        "Frequenz (Hz)",
                        "RMS (V RMS)",
                        "Leistung (W)",
                        "THD+N (%)",
                        "THD (%)",
                        "Status",
                    ] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for (index, record) in
                        run.demonstration.sweep.dataset.records.iter().enumerate()
                    {
                        let selected = self.selected_point == Some(index);
                        if ui
                            .selectable_label(selected, format!("{}", index + 1))
                            .clicked()
                        {
                            self.selected_point = Some(index);
                        }
                        ui.label(format_value(record.axis_value));
                        ui.label(format_optional_value(
                            run.demonstration.simulated_stimuli_vrms.get(index).copied(),
                        ));
                        ui.label(format_optional_value(
                            run.demonstration
                                .simulated_frequencies_hz
                                .get(index)
                                .copied(),
                        ));
                        ui.label(format_optional_value(measurement_value(
                            record,
                            "voltage.rms",
                        )));
                        ui.label(format_optional_value(
                            run.demonstration.measured_power_w(index),
                        ));
                        ui.label(format_optional_value(measurement_value(
                            record,
                            "distortion.thd+n",
                        )));
                        ui.label(format_optional_value(measurement_value(
                            record,
                            "distortion.thd",
                        )));
                        ui.label(record_status(record));
                        ui.end_row();
                    }
                });
        });

        if let Some(index) = self.selected_point {
            if let Some(point) = run
                .demonstration
                .points
                .iter()
                .find(|point| point.record_index == index)
            {
                ui.group(|ui| {
                    ui.label(format!("Messpunkt {} — Detail", index + 1));
                    ui.label(format!(
                        "Simulierter Generator-Level: {} V RMS",
                        format_value(point.generator_level_vrms)
                    ));
                    ui.label(format!(
                        "Gemessener RMS-Wert: {} V RMS",
                        format_value(point.measured_vrms)
                    ));
                    if let Some(power_w) = run.demonstration.measured_power_w(index) {
                        ui.label(format!(
                            "Gemessene Mock-Lastleistung: {} W",
                            format_value(power_w)
                        ));
                    }
                    ui.label(format!("THD+N: {} %", format_value(point.thd_n_percent)));
                    if let Some(thd) = point.thd_percent {
                        ui.label(format!("THD: {} %", format_value(thd)));
                    }
                });
            }
        }

        self.show_plots(ui, &run.demonstration);

        ui.horizontal(|ui| {
            ui.label("Export-Datei-Stamm:");
            ui.text_edit_singleline(&mut self.output_path);
            if ui.button("CSV exportieren").clicked() {
                self.export(false);
            }
            if ui.button("JSON exportieren").clicked() {
                self.export(true);
            }
        });
    }

    fn show_plots(&self, ui: &mut egui::Ui, result: &DemonstrationResult) {
        let voltage = self.show_rms.then_some(YMetric::MeasuredVrms);
        let distortion: Vec<_> = [
            self.show_thd_n.then_some(YMetric::ThdN),
            self.show_thd.then_some(YMetric::Thd),
        ]
        .into_iter()
        .flatten()
        .collect();

        if let Some(metric) = voltage {
            render_plot(
                ui,
                result,
                PlotOptions {
                    id: "voltage_plot",
                    title: "Voltage (V RMS)",
                    x_metric: self.x_metric,
                    y_metrics: &[metric],
                    log_x: self.log_x,
                    log_y: self.log_y,
                },
            );
        }
        if !distortion.is_empty() {
            render_plot(
                ui,
                result,
                PlotOptions {
                    id: "distortion_plot",
                    title: "Distortion (%)",
                    x_metric: self.x_metric,
                    y_metrics: &distortion,
                    log_x: self.log_x,
                    log_y: self.log_y,
                },
            );
        }
        if voltage.is_none() && distortion.is_empty() {
            ui.label("Wähle mindestens eine Messkurve aus.");
        }
    }

    fn show_dashboard(
        &mut self,
        ui: &mut egui::Ui,
    ) -> egui::containers::scroll_area::ScrollAreaOutput<()> {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading("ATS-2 — native Entwicklungs-GUI");
                ui.colored_label(egui::Color32::YELLOW, SIMULATION_NOTICE);
                self.show_controls(ui);
                self.show_results(ui);
                ui.separator();
                ui.label(&self.status);
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            })
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive_worker_events();
        egui::CentralPanel::default().show(context, |ui| {
            self.show_dashboard(ui);
        });
        if self.worker.is_some() {
            context.request_repaint_after(Duration::from_millis(30));
        }
    }
}

fn config_point_count(spacing: &SweepSpacing, configured_count: usize) -> usize {
    if *spacing == SweepSpacing::Fixture {
        3
    } else {
        configured_count
    }
}

fn format_value(value: f64) -> String {
    format!("{value:.6}")
}

fn format_optional_value(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_owned(), format_value)
}

fn measurement_value(
    record: &ats2_rs::sweep::MultiSweepRecord<f64, ats2_rs::sweep::demo::DemoError>,
    id: &str,
) -> Option<f64> {
    record
        .measurements
        .iter()
        .find(|measurement| measurement.id == id)?
        .result
        .as_ref()
        .ok()
        .copied()
        .filter(|value| value.is_finite())
}

fn record_status(
    record: &ats2_rs::sweep::MultiSweepRecord<f64, ats2_rs::sweep::demo::DemoError>,
) -> &'static str {
    if record
        .measurements
        .iter()
        .any(|measurement| measurement.result.is_err())
    {
        "Fehler · Mock"
    } else {
        "numerisch · Mock"
    }
}

fn plot_points(
    result: &DemonstrationResult,
    x_metric: XMetric,
    y_metric: YMetric,
    log_x: bool,
    log_y: bool,
) -> Vec<[f64; 2]> {
    result
        .sweep
        .dataset
        .records
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            let x = match x_metric {
                XMetric::SimulatedLevel => *result.simulated_stimuli_vrms.get(index)?,
                XMetric::FrequencyHz => *result.simulated_frequencies_hz.get(index)?,
                XMetric::MeasuredVrms => measurement_value(record, "voltage.rms")?,
                XMetric::MeasuredPowerW => result.measured_power_w(index)?,
            };
            let y = y_metric.value(record)?;
            if !x.is_finite()
                || !y.is_finite()
                || (x_metric == XMetric::MeasuredPowerW && x < 0.0)
                || (log_x && x <= 0.0)
                || (log_y && y <= 0.0)
            {
                return None;
            }
            Some([
                if log_x { x.log10() } else { x },
                if log_y { y.log10() } else { y },
            ])
        })
        .collect()
}

struct PlotOptions<'a> {
    id: &'static str,
    title: &'a str,
    x_metric: XMetric,
    y_metrics: &'a [YMetric],
    log_x: bool,
    log_y: bool,
}

fn render_plot(ui: &mut egui::Ui, result: &DemonstrationResult, options: PlotOptions<'_>) {
    let PlotOptions {
        id,
        title,
        x_metric,
        y_metrics,
        log_x,
        log_y,
    } = options;
    let x_label = match x_metric {
        XMetric::SimulatedLevel => "Simulated generator level (V RMS)",
        XMetric::FrequencyHz => "Frequency (Hz)",
        XMetric::MeasuredVrms => "Measured RMS (V RMS)",
        XMetric::MeasuredPowerW => "Measured output power (W; simulated resistive load)",
    };
    let x_label = if log_x {
        format!("log10({x_label})")
    } else {
        x_label.to_owned()
    };
    let y_label = if log_y {
        format!("log10({title})")
    } else {
        title.to_owned()
    };
    let mut plot = Plot::new(id)
        .legend(Legend::default())
        .x_axis_label(x_label)
        .y_axis_label(y_label)
        .height(250.0)
        .allow_zoom(true)
        .allow_drag(true);
    if log_x {
        plot = plot.x_axis_formatter(|mark, _range| format!("10^{:.2}", mark.value));
    }
    if log_y {
        plot = plot.y_axis_formatter(|mark, _range| format!("10^{:.2}", mark.value));
    }
    plot.show(ui, |plot_ui| {
        for metric in y_metrics {
            let points = plot_points(result, x_metric, *metric, log_x, log_y);
            if !points.is_empty() {
                plot_ui.line(Line::new(points).name(format!(
                    "{} ({})",
                    metric.label(),
                    metric.unit()
                )));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{XMetric, YMetric, plot_points};
    use ats2_rs::sweep::demo::simulate_demo;

    #[test]
    fn dashboard_scroll_region_keeps_export_controls_reachable_at_small_window() {
        let mut app = super::GuiApp {
            result: Some(
                super::simulate_demo_controlled(&super::DemoConfig::default(), || false, |_| {})
                    .unwrap(),
            ),
            ..super::GuiApp::default()
        };
        let context = eframe::egui::Context::default();
        let input = eframe::egui::RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(
                eframe::egui::Pos2::ZERO,
                eframe::egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut scroll_output = None;
        let _ = context.run(input, |context| {
            eframe::egui::CentralPanel::default().show(context, |ui| {
                scroll_output = Some(app.show_dashboard(ui));
            });
        });
        let scroll_output = scroll_output.expect("dashboard should render");
        assert!(scroll_output.content_size.y > scroll_output.inner_rect.height());
    }

    #[test]
    fn log_plot_uses_transformed_coordinates_without_mutating_saved_results() {
        let result = simulate_demo(None, true).unwrap();
        let original = result.points.clone();

        let points = plot_points(&result, XMetric::SimulatedLevel, YMetric::ThdN, true, false);

        assert_eq!(points.len(), result.points.len());
        assert_eq!(points[0][0], result.points[0].generator_level_vrms.log10());
        assert_eq!(points[0][1], result.points[0].thd_n_percent);
        assert_eq!(result.points, original);
    }

    #[test]
    fn plot_can_use_saved_measurement_as_x_without_remeasurement() {
        let result = simulate_demo(None, true).unwrap();

        let points = plot_points(&result, XMetric::MeasuredVrms, YMetric::Thd, false, false);

        assert_eq!(points.len(), result.points.len());
        assert_eq!(
            points[0],
            [
                result.points[0].measured_vrms,
                result.points[0].thd_percent.unwrap()
            ]
        );
    }

    #[test]
    fn thd_n_plot_can_use_explicitly_modeled_measured_power_as_x() {
        let config = super::DemoConfig {
            display_axis: super::DemoXAxis::MeasuredPowerW,
            load_resistance_ohms: Some(8.0),
            ..super::DemoConfig::default()
        };
        let run = super::simulate_demo_controlled(&config, || false, |_| {}).unwrap();
        let result = &run.demonstration;

        let points = plot_points(result, XMetric::MeasuredPowerW, YMetric::ThdN, false, false);

        assert_eq!(points.len(), result.points.len());
        assert_eq!(points[0][0], result.measured_power_w(0).unwrap());
        assert_eq!(points[0][1], result.points[0].thd_n_percent);
    }

    #[test]
    fn measured_power_plot_reads_dataset_and_handles_zero_and_invalid_log_values() {
        let mut result = simulate_demo(Some(8.0), true).unwrap();
        let power = result.sweep.dataset.records[0]
            .measurements
            .iter_mut()
            .find(|measurement| measurement.id == "power.w")
            .unwrap();
        power.result = Ok(0.0);

        let linear = plot_points(
            &result,
            XMetric::MeasuredPowerW,
            YMetric::ThdN,
            false,
            false,
        );
        assert_eq!(linear[0][0], 0.0);
        let logarithmic = plot_points(&result, XMetric::MeasuredPowerW, YMetric::ThdN, true, false);
        assert_eq!(logarithmic.len(), result.points.len() - 1);

        let power = result.sweep.dataset.records[1]
            .measurements
            .iter_mut()
            .find(|measurement| measurement.id == "power.w")
            .unwrap();
        power.result = Ok(f64::NAN);
        let linear = plot_points(
            &result,
            XMetric::MeasuredPowerW,
            YMetric::ThdN,
            false,
            false,
        );
        assert_eq!(linear.len(), result.points.len() - 1);

        let power = result.sweep.dataset.records[2]
            .measurements
            .iter_mut()
            .find(|measurement| measurement.id == "power.w")
            .unwrap();
        power.result = Ok(-0.1);
        let linear = plot_points(
            &result,
            XMetric::MeasuredPowerW,
            YMetric::ThdN,
            false,
            false,
        );
        assert_eq!(linear.len(), 1);
    }
}
