//! The [`HelmPort`] implementation of [`HelmCli`].

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{
    ChartHit, ChartRef, HelmCapabilities, InstallRequest, Release, ReleaseRef, RepoEntry, Revision,
    RollbackRequest, SearchQuery, ShowKind, UninstallRequest, UpgradeRequest, Values,
};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::{HelmOutput, HelmPort};

use super::HelmCli;
use super::args::{self, Get};
use super::errors::Redactor;
use super::parse;
use super::run::{Invocation, finish, run, stream};

/// What `repo list` and `search repo` say when no repository is configured (Helm 3 exits 1
/// for these; an empty list is the answer).
const NO_REPOS: &[&str] = &["no repositories to show", "no repositories configured"];

/// The values YAML for stdin, `None` when there are none (no `--values=-` then).
fn values_input(values: &Values) -> OxiResult<Option<String>> {
    if values.is_empty() {
        Ok(None)
    } else {
        Ok(Some(values.to_yaml()?))
    }
}

impl HelmCli {
    fn timeouts(&self) -> super::HelmTimeouts {
        self.config.read().timeouts
    }

    async fn get(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
        what: Get,
    ) -> OxiResult<String> {
        let target = self.target(&release.cluster)?;
        let argv = args::get(&target, &release.namespace, &release.name, revision, what);
        self.output(argv, self.timeouts().read, Some(&release.name))
            .await
    }

    /// A mutation's invocation: its deadline is Helm's own `--timeout` plus the grace.
    fn mutation(
        &self,
        argv: Vec<String>,
        timeout: std::time::Duration,
        values: Option<&Values>,
        release: &str,
    ) -> OxiResult<Invocation> {
        let mut invocation = Invocation::new(argv, timeout + self.timeouts().mutation_grace);
        invocation.release = Some(release.to_owned());
        if let Some(values) = values {
            invocation.stdin = values_input(values)?;
            invocation.redactor = Redactor::for_values(values);
        }
        Ok(invocation)
    }
}

#[async_trait]
impl HelmPort for HelmCli {
    async fn list_releases(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<Vec<Release>> {
        let target = self.target(cluster)?;
        let read = self.timeouts().read;
        let scopes: Vec<Option<&str>> = match namespaces {
            NamespaceSelection::All => vec![None],
            NamespaceSelection::Set(names) => names.iter().map(|n| Some(n.as_str())).collect(),
        };
        // One `helm list` per selected namespace, run side by side.
        let lists = scopes.into_iter().map(|scope| {
            let argv = args::list(&target, scope);
            async move {
                let out = self.output(argv, read, None).await?;
                Ok::<_, oxikube_domain::OxiError>(parse::releases(cluster, &out)?)
            }
        });
        let lists = futures::future::try_join_all(lists).await?;
        Ok(lists.into_iter().flatten().collect())
    }

    async fn history(&self, release: &ReleaseRef) -> OxiResult<Vec<Revision>> {
        let target = self.target(&release.cluster)?;
        let argv = args::history(&target, &release.namespace, &release.name);
        let out = self
            .output(argv, self.timeouts().read, Some(&release.name))
            .await?;
        Ok(parse::history(&out)?)
    }

    async fn values(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
        all: bool,
    ) -> OxiResult<Values> {
        let out = self.get(release, revision, Get::Values { all }).await?;
        Ok(parse::values(&out)?)
    }

    async fn manifest(&self, release: &ReleaseRef, revision: Option<u32>) -> OxiResult<String> {
        self.get(release, revision, Get::Manifest).await
    }

    async fn notes(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
    ) -> OxiResult<Option<String>> {
        let out = self.get(release, revision, Get::Notes).await?;
        Ok(parse::notes(&out))
    }

    async fn repos(&self) -> OxiResult<Vec<RepoEntry>> {
        let (binary, _) = self.binary().await?;
        match run(
            &binary,
            Invocation::new(args::repo_list(), self.timeouts().read),
        )
        .await
        {
            Ok(out) => Ok(parse::repos(&out)?),
            Err(failure) if NO_REPOS.iter().any(|n| failure.says(n)) => Ok(Vec::new()),
            Err(failure) => Err(failure.into_error(None)),
        }
    }

    async fn search(&self, query: &SearchQuery) -> OxiResult<Vec<ChartHit>> {
        let (binary, _) = self.binary().await?;
        match run(
            &binary,
            Invocation::new(args::search(query), self.timeouts().read),
        )
        .await
        {
            Ok(out) => Ok(parse::hits(&out)?),
            Err(failure) if NO_REPOS.iter().any(|n| failure.says(n)) => Ok(Vec::new()),
            Err(failure) => Err(failure.into_error(None)),
        }
    }

    async fn show(&self, chart: &ChartRef, kind: ShowKind) -> OxiResult<String> {
        self.output(args::show(chart, kind), self.timeouts().fetch, None)
            .await
    }

    async fn template(&self, request: &InstallRequest) -> OxiResult<String> {
        let (binary, _) = self.binary().await?;
        let stdin = values_input(&request.values)?;
        let mut invocation = Invocation::new(
            args::template(request, stdin.is_some()),
            self.timeouts().fetch,
        );
        invocation.stdin = stdin;
        invocation.redactor = Redactor::for_values(&request.values);
        finish(run(&binary, invocation).await, Some(&request.release))
    }

    async fn install(&self, request: &InstallRequest) -> OxiResult<HelmOutput> {
        let (binary, version) = self.binary().await?;
        let target = self.target(&request.cluster)?;
        let has_values = !request.values.is_empty();
        let argv = args::install(&target, request, has_values, version);
        let invocation = self.mutation(
            argv,
            request.timeout,
            Some(&request.values),
            &request.release,
        )?;
        Ok(stream(binary, invocation))
    }

    async fn upgrade(&self, request: &UpgradeRequest) -> OxiResult<HelmOutput> {
        let (binary, version) = self.binary().await?;
        let target = self.target(&request.release.cluster)?;
        let has_values = !request.values.is_empty();
        let argv = args::upgrade(&target, request, has_values, version);
        let name = &request.release.name;
        let invocation = self.mutation(argv, request.timeout, Some(&request.values), name)?;
        Ok(stream(binary, invocation))
    }

    async fn rollback(&self, request: &RollbackRequest) -> OxiResult<()> {
        let (binary, _) = self.binary().await?;
        let target = self.target(&request.release.cluster)?;
        let name = &request.release.name;
        let invocation = self.mutation(
            args::rollback(&target, request),
            request.timeout,
            None,
            name,
        )?;
        finish(run(&binary, invocation).await, Some(name)).map(drop)
    }

    async fn uninstall(&self, request: &UninstallRequest) -> OxiResult<()> {
        let (binary, _) = self.binary().await?;
        let target = self.target(&request.release.cluster)?;
        let name = &request.release.name;
        let argv = args::uninstall(&target, request);
        let invocation = self.mutation(argv, request.timeout, None, name)?;
        finish(run(&binary, invocation).await, Some(name)).map(drop)
    }

    fn capabilities(&self) -> HelmCapabilities {
        self.detection.read().capabilities()
    }
}
