use super::SerializedAxis;
use crate::{item::ItemHandle, ItemDeserializers, Member, Pane, PaneAxis, Workspace, WorkspaceId};
use anyhow::{Context, Result};
use async_recursion::async_recursion;
use client::RemoteProjectId;
use db::sqlez::{
    bindable::{Bind, Column, StaticColumnCount},
    statement::Statement,
};
use gpui::{AppContext, AsyncWindowContext, Bounds, DevicePixels, Model, Task, View, WeakView};
use project::Project;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use util::ResultExt;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SerializedRemoteProject {
    id: RemoteProjectId,
    dev_server_name: String,
}

#[derive(Debug, PartialEq, Clone)]
pub struct WorkspaceLocation {
    paths: Arc<Vec<PathBuf>>,
    remote_project: Option<SerializedRemoteProject>,
}

impl WorkspaceLocation {
    pub fn local<P: AsRef<Path>>(paths: impl IntoIterator<Item = P>) -> Self {
        Self {
            paths: Arc::new(
                paths
                    .into_iter()
                    .map(|p| p.as_ref().to_path_buf())
                    .collect(),
            ),
            remote_project: None,
        }
    }

    pub fn remote(remote_project_id: RemoteProjectId, cx: &AppContext) -> Self {
        let store = remote_projects::Store::global(cx).read(cx);

        let (paths, dev_server_name) =
            if let Some(remote_project) = store.find_remote_project_by_id(remote_project_id) {
                (
                    vec![remote_project.path.to_string().into()],
                    store
                        .dev_server(remote_project.dev_server_id)
                        .map(|dev_server| dev_server.name.to_string()),
                )
            } else {
                (vec![], None)
            };

        Self {
            paths: Arc::new(paths),
            remote_project: Some(SerializedRemoteProject {
                id: remote_project_id,
                dev_server_name: dev_server_name.unwrap_or_default(),
            }),
        }
    }

    pub fn paths(&self) -> Arc<Vec<PathBuf>> {
        self.paths.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_local_paths<P: AsRef<Path>>(paths: Vec<P>) -> Self {
        Self {
            paths: Arc::new(
                paths
                    .into_iter()
                    .map(|p| p.as_ref().to_path_buf())
                    .collect(),
            ),
            remote_project: None,
        }
    }
}

// impl<P: AsRef<Path>, T: IntoIterator<Item = P>> From<T> for WorkspaceLocation {
//     fn from(iterator: T) -> Self {
//         let mut roots = iterator
//             .into_iter()
//             .map(|p| p.as_ref().to_path_buf())
//             .collect::<Vec<_>>();
//         roots.sort();
//         Self::local(Arc::new(roots))
//     }
// }

impl StaticColumnCount for WorkspaceLocation {
    fn column_count() -> usize {
        2
    }
}

impl Bind for &WorkspaceLocation {
    fn bind(&self, statement: &Statement, start_index: i32) -> Result<i32> {
        bincode::serialize(&self.paths)
            .expect("Bincode serialization of paths should not fail")
            .bind(statement, start_index)
            .map_err(|e| dbg!(e))?;
        dbg!(serde_json::to_string(&self.remote_project)
            .expect("Json serialization of remote project should not fail"))
        .bind(statement, start_index + 1)
        .map_err(|e| dbg!(e))
        .map(|ret| dbg!(ret))
    }
}

impl Column for WorkspaceLocation {
    fn column(statement: &mut Statement, start_index: i32) -> Result<(Self, i32)> {
        dbg!("wut?");
        let path_blob = statement.column_blob(start_index)?;
        let paths =
            bincode::deserialize(path_blob).context("Bincode deserialization of paths failed")?;
        let dev_server: Option<SerializedRemoteProject> = statement
            .column_text(start_index + 1)
            .and_then(|dev_server_json| Ok(serde_json::from_str(dev_server_json)?))
            .context("Deserialization of remote project json failed")?;

        Ok((
            WorkspaceLocation {
                paths,
                remote_project: dev_server,
            },
            start_index + 2,
        ))
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SerializedWorkspace {
    pub(crate) id: WorkspaceId,
    pub(crate) location: WorkspaceLocation,
    pub(crate) center_group: SerializedPaneGroup,
    pub(crate) bounds: Option<Bounds<DevicePixels>>,
    pub(crate) fullscreen: bool,
    pub(crate) display: Option<Uuid>,
    pub(crate) docks: DockStructure,
}

#[derive(Debug, PartialEq, Clone, Default)]
pub struct DockStructure {
    pub(crate) left: DockData,
    pub(crate) right: DockData,
    pub(crate) bottom: DockData,
}

impl Column for DockStructure {
    fn column(statement: &mut Statement, start_index: i32) -> Result<(Self, i32)> {
        let (left, next_index) = dbg!(DockData::column(statement, start_index))?;
        let (right, next_index) = dbg!(DockData::column(statement, next_index))?;
        let (bottom, next_index) = dbg!(DockData::column(statement, next_index))?;
        Ok((
            DockStructure {
                left,
                right,
                bottom,
            },
            next_index,
        ))
    }
}

impl Bind for DockStructure {
    fn bind(&self, statement: &Statement, start_index: i32) -> Result<i32> {
        let next_index = dbg!(statement.bind(&self.left, start_index))?;
        let next_index = dbg!(statement.bind(&self.right, next_index))?;
        statement.bind(&self.bottom, next_index)
    }
}

#[derive(Debug, PartialEq, Clone, Default)]
pub struct DockData {
    pub(crate) visible: bool,
    pub(crate) active_panel: Option<String>,
    pub(crate) zoom: bool,
}

impl Column for DockData {
    fn column(statement: &mut Statement, start_index: i32) -> Result<(Self, i32)> {
        let (visible, next_index) = Option::<bool>::column(statement, start_index)?;
        let (active_panel, next_index) = Option::<String>::column(statement, next_index)?;
        let (zoom, next_index) = Option::<bool>::column(statement, next_index)?;
        Ok((
            DockData {
                visible: visible.unwrap_or(false),
                active_panel,
                zoom: zoom.unwrap_or(false),
            },
            next_index,
        ))
    }
}

impl Bind for DockData {
    fn bind(&self, statement: &Statement, start_index: i32) -> Result<i32> {
        let next_index = statement.bind(&self.visible, start_index)?;
        let next_index = statement.bind(&self.active_panel, next_index)?;
        statement.bind(&self.zoom, next_index)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum SerializedPaneGroup {
    Group {
        axis: SerializedAxis,
        flexes: Option<Vec<f32>>,
        children: Vec<SerializedPaneGroup>,
    },
    Pane(SerializedPane),
}

#[cfg(test)]
impl Default for SerializedPaneGroup {
    fn default() -> Self {
        Self::Pane(SerializedPane {
            children: vec![SerializedItem::default()],
            active: false,
        })
    }
}

impl SerializedPaneGroup {
    #[async_recursion(?Send)]
    pub(crate) async fn deserialize(
        self,
        project: &Model<Project>,
        workspace_id: WorkspaceId,
        workspace: WeakView<Workspace>,
        cx: &mut AsyncWindowContext,
    ) -> Option<(Member, Option<View<Pane>>, Vec<Option<Box<dyn ItemHandle>>>)> {
        match self {
            SerializedPaneGroup::Group {
                axis,
                children,
                flexes,
            } => {
                let mut current_active_pane = None;
                let mut members = Vec::new();
                let mut items = Vec::new();
                for child in children {
                    if let Some((new_member, active_pane, new_items)) = child
                        .deserialize(project, workspace_id, workspace.clone(), cx)
                        .await
                    {
                        members.push(new_member);
                        items.extend(new_items);
                        current_active_pane = current_active_pane.or(active_pane);
                    }
                }

                if members.is_empty() {
                    return None;
                }

                if members.len() == 1 {
                    return Some((members.remove(0), current_active_pane, items));
                }

                Some((
                    Member::Axis(PaneAxis::load(axis.0, members, flexes)),
                    current_active_pane,
                    items,
                ))
            }
            SerializedPaneGroup::Pane(serialized_pane) => {
                let pane = workspace
                    .update(cx, |workspace, cx| workspace.add_pane(cx).downgrade())
                    .log_err()?;
                let active = serialized_pane.active;
                let new_items = serialized_pane
                    .deserialize_to(project, &pane, workspace_id, workspace.clone(), cx)
                    .await
                    .log_err()?;

                if pane.update(cx, |pane, _| pane.items_len() != 0).log_err()? {
                    let pane = pane.upgrade()?;
                    Some((Member::Pane(pane.clone()), active.then(|| pane), new_items))
                } else {
                    let pane = pane.upgrade()?;
                    workspace
                        .update(cx, |workspace, cx| workspace.force_remove_pane(&pane, cx))
                        .log_err()?;
                    None
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct SerializedPane {
    pub(crate) active: bool,
    pub(crate) children: Vec<SerializedItem>,
}

impl SerializedPane {
    pub fn new(children: Vec<SerializedItem>, active: bool) -> Self {
        SerializedPane { children, active }
    }

    pub async fn deserialize_to(
        &self,
        project: &Model<Project>,
        pane: &WeakView<Pane>,
        workspace_id: WorkspaceId,
        workspace: WeakView<Workspace>,
        cx: &mut AsyncWindowContext,
    ) -> Result<Vec<Option<Box<dyn ItemHandle>>>> {
        let mut item_tasks = Vec::new();
        let mut active_item_index = None;
        let mut preview_item_index = None;
        for (index, item) in self.children.iter().enumerate() {
            let project = project.clone();
            item_tasks.push(pane.update(cx, |_, cx| {
                if let Some(deserializer) = cx.global::<ItemDeserializers>().get(&item.kind) {
                    deserializer(project, workspace.clone(), workspace_id, item.item_id, cx)
                } else {
                    Task::ready(Err(anyhow::anyhow!(
                        "Deserializer does not exist for item kind: {}",
                        item.kind
                    )))
                }
            })?);
            if item.active {
                active_item_index = Some(index);
            }
            if item.preview {
                preview_item_index = Some(index);
            }
        }

        let mut items = Vec::new();
        for item_handle in futures::future::join_all(item_tasks).await {
            let item_handle = item_handle.log_err();
            items.push(item_handle.clone());

            if let Some(item_handle) = item_handle {
                pane.update(cx, |pane, cx| {
                    pane.add_item(item_handle.clone(), true, true, None, cx);
                })?;
            }
        }

        if let Some(active_item_index) = active_item_index {
            pane.update(cx, |pane, cx| {
                pane.activate_item(active_item_index, false, false, cx);
            })?;
        }

        if let Some(preview_item_index) = preview_item_index {
            pane.update(cx, |pane, cx| {
                if let Some(item) = pane.item_for_index(preview_item_index) {
                    pane.set_preview_item_id(Some(item.item_id()), cx);
                }
            })?;
        }

        anyhow::Ok(items)
    }
}

pub type GroupId = i64;
pub type PaneId = i64;
pub type ItemId = u64;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SerializedItem {
    pub kind: Arc<str>,
    pub item_id: ItemId,
    pub active: bool,
    pub preview: bool,
}

impl SerializedItem {
    pub fn new(kind: impl AsRef<str>, item_id: ItemId, active: bool, preview: bool) -> Self {
        Self {
            kind: Arc::from(kind.as_ref()),
            item_id,
            active,
            preview,
        }
    }
}

#[cfg(test)]
impl Default for SerializedItem {
    fn default() -> Self {
        SerializedItem {
            kind: Arc::from("Terminal"),
            item_id: 100000,
            active: false,
            preview: false,
        }
    }
}

impl StaticColumnCount for SerializedItem {
    fn column_count() -> usize {
        4
    }
}
impl Bind for &SerializedItem {
    fn bind(&self, statement: &Statement, start_index: i32) -> Result<i32> {
        let next_index = statement.bind(&self.kind, start_index)?;
        let next_index = statement.bind(&self.item_id, next_index)?;
        let next_index = statement.bind(&self.active, next_index)?;
        statement.bind(&self.preview, next_index)
    }
}

impl Column for SerializedItem {
    fn column(statement: &mut Statement, start_index: i32) -> Result<(Self, i32)> {
        let (kind, next_index) = Arc::<str>::column(statement, start_index)?;
        let (item_id, next_index) = ItemId::column(statement, next_index)?;
        let (active, next_index) = bool::column(statement, next_index)?;
        let (preview, next_index) = bool::column(statement, next_index)?;
        Ok((
            SerializedItem {
                kind,
                item_id,
                active,
                preview,
            },
            next_index,
        ))
    }
}
