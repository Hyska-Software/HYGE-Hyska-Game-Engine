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
            Label { text: "Dropped: " + editorBridge.droppedFrames; color: "#c4c9d4" }
            Button { text: "Connect"; onClicked: editorBridge.connect_backend() }
            Button { text: "Open Project"; onClicked: editorBridge.open_project() }
        }
    }

    SplitView {
        anchors.fill: parent

        Frame {
            SplitView.preferredWidth: 300
            ColumnLayout {
                anchors.fill: parent
                Label { text: "Hierarchy"; font.bold: true }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: hierarchyModel
                    clip: true
                    delegate: ItemDelegate {
                        width: ListView.view.width
                        text: model.name
                        highlighted: model.selected
                    }
                }
                Label { text: "Assets"; font.bold: true }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 180
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
                    Layout.preferredHeight: 150
                    model: consoleModel
                    clip: true
                    delegate: Label { width: ListView.view.width; text: model.display; color: "#c4c9d4"; elide: Text.ElideRight }
                }
            }
        }

        Frame {
            SplitView.preferredWidth: 340
            ColumnLayout {
                anchors.fill: parent
                Label { text: "Inspector"; font.bold: true }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: inspectorModel
                    clip: true
                    delegate: Column {
                        width: ListView.view.width
                        spacing: 3
                        Label { text: model.typePath; font.bold: true; elide: Text.ElideRight }
                        Label { text: model.error || JSON.stringify(model.value); color: model.error ? "#ed8b8b" : "#c4c9d4"; wrapMode: Text.Wrap }
                        Rectangle { width: parent.width; height: 1; color: "#363c47" }
                    }
                }
                Label { text: "Profiler"; font.bold: true }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 180
                    model: profilerModel
                    clip: true
                    delegate: Label { width: ListView.view.width; text: model.display; color: "#c4c9d4" }
                }
            }
        }
    }
}
