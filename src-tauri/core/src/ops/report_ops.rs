//! Reports: writing one as a job, and reading, answering and deleting saved ones.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Ctx, DeviceId, JobId, OpError};
use crate::reports::{ActionResponse, ActionStatus, CitedAnnotation, LatestVerdict, Report, ReportSummary, UserImpact};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerateReportRequest {
    /// Session files as `list_sessions` gives them.
    pub paths: Vec<String>,
    /// A model alias such as "sonnet"; None or empty is `reports::DEFAULT_MODEL`.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub focus: Option<String>,
    /// Analyze the sessions without an analysis first; otherwise they are left out.
    #[serde(default)]
    pub analyze_first: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelReportRequest {
    /// The report job's id.
    pub job: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListReportsRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetReportRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportAnnotationsRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteReportRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResponsesRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetActionResponseRequest {
    pub input: ActionResponseChange,
}

/// `reports::ActionResponseInput` on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResponseChange {
    pub report_id: String,
    pub action_index: usize,
    #[serde(default)]
    pub status: Option<ActionStatus>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub user_impact: Option<UserImpact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionVerdictsRequest {
    pub id: String,
}

/// Starts a Report job. Progress arrives as report-progress events and the end as one report-done
/// naming the saved report. BadRequest with no sessions; Conflict while another Report job runs.
pub(super) fn generate_report(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &GenerateReportRequest) -> Result<JobId, OpError> {
    todo!("generate_report")
}

/// Cancels the Report job and kills its `claude`.
pub(super) fn cancel_report(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &CancelReportRequest) -> Result<(), OpError> {
    todo!("cancel_report")
}

pub(super) fn list_reports(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ListReportsRequest) -> Result<Vec<ReportSummary>, OpError> {
    todo!("list_reports")
}

pub(super) fn get_report(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &GetReportRequest) -> Result<Option<Report>, OpError> {
    todo!("get_report")
}

/// The annotations a report was given, as it saw them, with their versions now.
pub(super) fn report_annotations(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ReportAnnotationsRequest) -> Result<Vec<CitedAnnotation>, OpError> {
    todo!("report_annotations")
}

pub(super) fn delete_report(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &DeleteReportRequest) -> Result<(), OpError> {
    todo!("delete_report")
}

pub(super) fn action_responses(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ActionResponsesRequest) -> Result<Vec<ActionResponse>, OpError> {
    todo!("action_responses")
}

/// Replaces the response to one action; None when nothing is left to keep.
pub(super) fn set_action_response(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &SetActionResponseRequest) -> Result<Option<ActionResponse>, OpError> {
    todo!("set_action_response")
}

/// The latest verdict later reports gave on each of this report's actions.
pub(super) fn action_verdicts(_ctx: &Arc<Ctx>, _device: &DeviceId, _req: &ActionVerdictsRequest) -> Result<Vec<LatestVerdict>, OpError> {
    todo!("action_verdicts")
}
