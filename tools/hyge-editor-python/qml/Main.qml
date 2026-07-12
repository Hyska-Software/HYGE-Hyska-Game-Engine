import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ApplicationWindow {
    visible: true
    width: 1440
    height: 900
    title: "Hyge Editor"
    color: "#20242b"

    property int viewportRevision: 0

    Connections {
        target: viewportController
        function onImageChanged() { viewportRevision += 1 }
    }

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            Label { text: "HYGE"; font.bold: true; font.pixelSize: 18 }
            Label { text: editorBridge.status; color: editorBridge.status === "Ready" ? "#9be7a5" : "#f2c879" }
            Item { Layout.fillWidth: true }
            Label { text: editorInteraction.hasConflict ? editorInteraction.conflictMessage : "Revision " + editorInteraction.revision; color: editorInteraction.hasConflict ? "#ed8b8b" : "#c4c9d4" }
            Label { text: "Dropped: " + editorBridge.droppedFrames; color: "#c4c9d4" }
            Button { text: "Connect"; onClicked: editorBridge.connect_backend() }
            Button { text: "Open Project"; onClicked: editorBridge.open_project() }
            Button { text: "Refresh"; onClicked: editorInteraction.refresh_snapshot() }
        }
    }

    SplitView {
        anchors.fill: parent

        Frame {
            SplitView.preferredWidth: 330
            ColumnLayout {
                anchors.fill: parent
                Label { text: "Hierarchy"; font.bold: true }
                TreeView {
                    id: hierarchyTree
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: hierarchyModel
                    clip: true
                    delegate: ItemDelegate {
                        required property int row
                        required property int depth
                        required property bool isTreeNode
                        required property bool expanded
                        width: hierarchyTree.width
                        leftPadding: 8 + depth * 16
                        text: model.name
                        highlighted: model.selected
                        onExpandedChanged: hierarchyModel.set_expanded(model.entity, expanded)
                        Drag.active: dragHandler.active
                        Drag.keys: ["hyge-entity"]
                        Drag.source: parent
                        Drag.mimeData: { "text/plain": String(model.entity) }

                        DragHandler { id: dragHandler; xAxis.enabled: false; yAxis.enabled: false }
                        MouseArea {
                            anchors.fill: parent
                            acceptedButtons: Qt.LeftButton
                            onClicked: hierarchyModel.select_entity(model.entity, (mouse.modifiers & Qt.ShiftModifier) !== 0)
                        }
                        DropArea {
                            anchors.fill: parent
                            keys: ["hyge-entity"]
                            onDropped: {
                                if (drag.source && drag.source.model && drag.source.model.entity !== model.entity)
                                    hierarchyModel.reparent_entity(drag.source.model.entity, model.entity)
                            }
                        }
                    }
                }
                Label { text: "Assets"; font.bold: true }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 160
                    model: assetModel
                    clip: true
                    delegate: Label { width: ListView.view.width; text: model.path; elide: Text.ElideMiddle }
                }
            }
        }

        Frame {
            SplitView.fillWidth: true
            SplitView.fillHeight: true
            ColumnLayout {
                anchors.fill: parent
                Label { text: "Viewport"; font.bold: true }
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: "#101216"
                    Image {
                        anchors.fill: parent
                        fillMode: Image.PreserveAspectFit
                        source: "image://hyge-viewport/frame?" + viewportRevision
                        asynchronous: true
                    }
                    Label {
                        anchors.centerIn: parent
                        visible: viewportController.state !== "connected"
                        text: "Waiting for viewport frame"
                        color: "#9aa1ad"
                    }
                }
                Label { text: "Console"; font.bold: true }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 140
                    model: consoleModel
                    clip: true
                    delegate: Label { width: ListView.view.width; text: model.display; color: "#c4c9d4"; elide: Text.ElideRight }
                }
            }
        }

        Frame {
            SplitView.preferredWidth: 390
            ColumnLayout {
                anchors.fill: parent
                Label { text: "Inspector"; font.bold: true }
                TreeView {
                    id: inspectorTree
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: inspectorModel
                    clip: true
                    delegate: ColumnLayout {
                        required property int depth
                        required property bool isTreeNode
                        width: inspectorTree.width
                        spacing: 2
                        RowLayout {
                            Layout.fillWidth: true
                            Layout.leftMargin: 8 + depth * 14
                            Label { text: model.label; font.bold: model.nodeKind === "component"; Layout.preferredWidth: 120; elide: Text.ElideRight }
                            Label { text: model.mixed ? "Multiple Values" : model.displayValue; color: model.error ? "#ed8b8b" : "#c4c9d4"; Layout.fillWidth: true; elide: Text.ElideRight }
                            TextField {
                                visible: model.editable && model.editorKind !== "group"
                                Layout.preferredWidth: 150
                                text: model.mixed ? "" : model.displayValue
                                placeholderText: model.mixed ? "Multiple Values" : ""
                                onTextChanged: {
                                    if (!activeFocus) return
                                    var value = text
                                    if (model.editorKind === "number") value = Number(text)
                                    else if (model.editorKind === "bool") value = text === "true"
                                    else if (model.editorKind === "vector" || model.editorKind === "quaternion") {
                                        try { value = JSON.parse(text) } catch (error) { return }
                                    }
                                    inspectorModel.edit_field(model.fieldId, value)
                                }
                            }
                        }
                        Label { visible: model.error !== ""; text: model.error; color: "#ed8b8b"; wrapMode: Text.Wrap; Layout.leftMargin: 8 + depth * 14 }
                    }
                }
                Label { text: "Profiler"; font.bold: true }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 160
                    model: profilerModel
                    clip: true
                    delegate: Label { width: ListView.view.width; text: model.display; color: "#c4c9d4" }
                }
            }
        }
    }
}
