import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ApplicationWindow {
    visible: true
    width: 1280
    height: 720
    title: "Hyge Editor"
    color: "#20242b"

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        ToolBar {
            Layout.fillWidth: true
            RowLayout {
                anchors.fill: parent
                Label { text: "HYGE"; font.bold: true }
                Label { text: editorBridge.status; color: "#9bd5ff" }
                Item { Layout.fillWidth: true }
                Button { text: "Open Project"; onClicked: editorBridge.open_project() }
            }
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true

            Frame {
                SplitView.preferredWidth: 250
                ColumnLayout {
                    anchors.fill: parent
                    Label { text: "Hierarchy"; font.bold: true }
                    Label { text: "Connected to Rust editor service"; wrapMode: Text.WordWrap }
                }
            }

            Frame {
                SplitView.fillWidth: true
                SplitView.fillHeight: true
                Label { anchors.centerIn: parent; text: "Viewport\n(RenderGraph integration pending)"; horizontalAlignment: Text.AlignHCenter }
            }

            Frame {
                SplitView.preferredWidth: 300
                Label { anchors.centerIn: parent; text: "Inspector" }
            }
        }
    }
}
