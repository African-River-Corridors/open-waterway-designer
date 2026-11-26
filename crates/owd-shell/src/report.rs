//! The reports of a design for the app's Report button (spec 0014, ticket 6).
//!
//! The reports come from `owd-report`, the crate behind `owd report`, with the same request, so a
//! report the app writes has the bytes `owd report` writes for the same design, option, kind,
//! format and document control. The front end only picks the kind, the format and the control.

use owd_report::{Control, Format, Kind, Request, extension};
use serde_json::Value;

/// What the user asks for: one kind or `all`, one format or `all`, and document control.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportAsk {
    pub kind: String,
    pub format: String,
    pub revision: Option<String>,
    pub status: Option<String>,
    pub prepared: Option<String>,
    /// ISO 8601: the front end gives today.
    pub date: String,
    /// What asked: the app and its platform, for the run report.
    pub requested_by: String,
}

impl ReportAsk {
    /// Read the ask from the JSON the front end sends: `{kind, format, revision?, status?,
    /// prepared?, date}`.
    pub fn from_json(v: &Value, requested_by: &str) -> Result<ReportAsk, String> {
        let s = |k: &str| {
            v[k].as_str()
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(String::from)
        };
        Ok(ReportAsk {
            kind: s("kind").unwrap_or_else(|| "screening".into()),
            format: s("format").unwrap_or_else(|| "pdf".into()),
            revision: s("revision"),
            status: s("status"),
            prepared: s("prepared"),
            date: s("date").ok_or("the report needs a date")?,
            requested_by: requested_by.to_string(),
        })
    }
}

/// The inputs of the reports, taken under the lock so the reports are made off it.
pub struct ReportJob {
    design: String,
    dem: Option<owd_format::sectioned::DemFile>,
    option: Option<String>,
    ask: ReportAsk,
    /// What the design names by reference and the report reads (BLD-1).
    extras: owd_report::Extras,
}

impl ReportJob {
    /// The report reads these too (BLD-1: the buildings and the channel).
    pub(crate) fn with_extras(mut self, extras: owd_report::Extras) -> ReportJob {
        self.extras = extras;
        self
    }

    pub(crate) fn new(
        design: String,
        dem: Option<owd_format::sectioned::DemFile>,
        option: Option<&str>,
        ask: ReportAsk,
    ) -> ReportJob {
        ReportJob {
            design,
            dem,
            option: option.map(String::from),
            ask,
            extras: owd_report::Extras::default(),
        }
    }

    /// Each file, named as `owd report` names it (`<document number in lower case>.<ext>`), in the
    /// order `owd report` writes them.
    pub fn files(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        let kinds: Vec<Kind> = match self.ask.kind.as_str() {
            "all" => Kind::ALL.to_vec(),
            w => vec![Kind::from_word(w).ok_or_else(|| format!("`{w}` is not a kind of report"))?],
        };
        let formats: Vec<Format> = match self.ask.format.as_str() {
            "all" => Format::ALL.to_vec(),
            w => vec![Format::parse(w).ok_or_else(|| format!("`{w}` is not a format"))?],
        };
        let mut control = Control::draft(&self.ask.date);
        if let Some(r) = &self.ask.revision {
            control.revision = r.clone();
        }
        if let Some(s) = &self.ask.status {
            control.status = s.clone();
        }
        control.prepared = self.ask.prepared.clone();
        let mut out = Vec::new();
        for kind in kinds {
            let req = Request {
                kind,
                option: self.option.clone(),
                control: control.clone(),
                requested_by: Some(self.ask.requested_by.clone()),
            };
            let report =
                owd_report::build_with(&self.design, self.dem.as_ref(), &req, &self.extras)
                    .map_err(|e| e.to_string())?;
            for &f in &formats {
                let r = report.render(f).map_err(|e| e.to_string())?;
                out.push((format!("{}.{}", report.stem(), extension(f)), r.bytes));
            }
        }
        Ok(out)
    }
}
