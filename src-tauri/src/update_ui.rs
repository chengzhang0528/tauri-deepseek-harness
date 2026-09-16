use crate::official::compare_versions;
use crate::runtime::{
    AvailableUpdate, CheckResult, RuntimeCopy, RuntimeUpdateSettings, UpdateSourcePolicy,
};
use std::fmt::Write as _;

#[derive(Debug)]
pub enum UpdateOffer {
    Download(AvailableUpdate),
    Install(RuntimeCopy),
}

impl UpdateOffer {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Download(_) => "下载更新",
            Self::Install(_) => "重启并更新…",
        }
    }
}

pub struct UpdateView {
    pub heading: &'static str,
    pub description: String,
    pub offer: Option<UpdateOffer>,
}

impl UpdateView {
    pub fn new(
        running: &str,
        current: Option<&RuntimeCopy>,
        staged: Option<&RuntimeCopy>,
        check: &CheckResult,
        settings: &RuntimeUpdateSettings,
    ) -> Self {
        let prepared = staged.filter(|copy| {
            copy.harness_version.as_ref().is_some_and(|version| {
                current
                    .and_then(|active| active.harness_version.as_ref())
                    .is_none_or(|active| compare_versions(version, active).is_gt())
            })
        });
        let offer = match &check.available {
            Some(target) => match prepared.filter(|copy| {
                copy.harness_version.as_ref() == Some(&target.version)
                    && copy.source == target.source
                    && copy.registry == target.registry
            }) {
                Some(copy) => Some(UpdateOffer::Install(copy.clone())),
                None => Some(UpdateOffer::Download(target.clone())),
            },
            None => prepared.map(|copy| UpdateOffer::Install(copy.clone())),
        };
        let mut description = format!(
            "当前运行：DeepSeek Harness {running}\n更新来源：{}\n版本选择：{}",
            settings.source.label(),
            settings.version.as_deref().unwrap_or("自动选择最新版本")
        );
        let heading = match &offer {
            Some(UpdateOffer::Download(target)) => {
                let _ = write!(
                    description,
                    "\n\n可更新至 {}。下载期间可以继续使用；完成后再确认重启。",
                    target.version
                );
                "有可用更新"
            }
            Some(UpdateOffer::Install(copy)) => {
                let _ = write!(
                    description,
                    "\n\n已下载 {}。重启前会提示任务中断风险，并再次请你确认。",
                    copy.version_label()
                );
                if check.available.is_none() {
                    description.push_str("\n这是之前下载的版本，尚未安装。");
                }
                "更新已下载"
            }
            None if !check.issues.is_empty() => "未能完成更新检查",
            None if settings.version.is_some() => {
                description.push_str("\n\n当前固定版本没有可安装的更新。若要查找新版本，请在“更新设置”中选择自动选择最新版本。");
                "正在使用固定版本"
            }
            None if matches!(
                settings.source,
                UpdateSourcePolicy::Local | UpdateSourcePolicy::Oss
            ) =>
            {
                description.push_str("\n\n本机没有更高版本。若要联网检查，请在“更新设置”中选择自动选择或官方软件源。");
                "本机没有可用更新"
            }
            None if check.versions.is_empty() => {
                description.push_str("\n\n该来源未返回可用版本。请检查更新设置后重试。");
                "未找到可用版本"
            }
            None => {
                description.push_str("\n\n当前来源没有更高版本，无需更新。");
                "已是当前来源的最新版本"
            }
        };
        if !check.issues.is_empty() {
            let _ = write!(
                description,
                "\n\n部分检查未完成：{}\n可以稍后重新检查，或在“更新设置”中调整来源。",
                check.issues.join("；")
            );
        }
        Self {
            heading,
            description,
            offer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeSource;

    fn copy(version: &str) -> RuntimeCopy {
        RuntimeCopy {
            schema: 2,
            location: "test".into(),
            release: version.into(),
            harness_version: Some(version.into()),
            source: RuntimeSource::Npm,
            publisher: "@deepseek-ai/dsh".into(),
            registry: None,
        }
    }
    fn target(version: &str) -> AvailableUpdate {
        AvailableUpdate {
            version: version.into(),
            source: RuntimeSource::Npm,
            registry: None,
            local_copy: None,
        }
    }
    #[test]
    fn failed_check_never_claims_latest_or_offers_download() {
        let check = CheckResult {
            issues: vec!["来源不可用".into()],
            ..Default::default()
        };
        let view = UpdateView::new(
            "1.0.0",
            Some(&copy("1.0.0")),
            None,
            &check,
            &RuntimeUpdateSettings::default(),
        );
        assert_eq!(view.heading, "未能完成更新检查");
        assert!(view.offer.is_none());
    }
    #[test]
    fn fixed_version_and_local_only_do_not_claim_global_latest() {
        let mut settings = RuntimeUpdateSettings {
            version: Some("1.0.0".into()),
            ..Default::default()
        };
        let check = CheckResult {
            versions: vec![target("1.0.0")],
            ..Default::default()
        };
        assert_eq!(
            UpdateView::new("1.0.0", None, None, &check, &settings).heading,
            "正在使用固定版本"
        );
        settings.version = None;
        settings.source = UpdateSourcePolicy::Local;
        assert_eq!(
            UpdateView::new("1.0.0", None, None, &check, &settings).heading,
            "本机没有可用更新"
        );
    }
    #[test]
    fn only_matching_prepared_target_skips_download() {
        let check = CheckResult {
            available: Some(target("1.2.0")),
            ..Default::default()
        };
        let settings = RuntimeUpdateSettings::default();
        let active = copy("1.0.0");
        let view = UpdateView::new(
            "1.0.0",
            Some(&active),
            Some(&copy("1.1.0")),
            &check,
            &settings,
        );
        assert!(matches!(view.offer, Some(UpdateOffer::Download(_))));
        let view = UpdateView::new(
            "1.0.0",
            Some(&active),
            Some(&copy("1.2.0")),
            &check,
            &settings,
        );
        assert!(matches!(view.offer, Some(UpdateOffer::Install(_))));
    }
    #[test]
    fn previously_downloaded_update_remains_explicit_and_active_copy_is_not_offered() {
        let settings = RuntimeUpdateSettings::default();
        let active = copy("1.0.0");
        let check = CheckResult::default();
        let view = UpdateView::new(
            "1.0.0",
            Some(&active),
            Some(&copy("1.1.0")),
            &check,
            &settings,
        );
        assert!(matches!(view.offer, Some(UpdateOffer::Install(_))));
        assert!(view.description.contains("之前下载"));
        let view = UpdateView::new("1.0.0", Some(&active), Some(&active), &check, &settings);
        assert!(view.offer.is_none());
    }
}
