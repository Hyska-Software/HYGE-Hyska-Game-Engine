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
                    delegate: ItemDelegate {
                        width: ListView.view.width
                        text: model.name + "  (" + model.kind + ")"
                        highlighted: model.selected
                        onClicked: assetModel.select_asset(model.assetId)
                        onDoubleClicked: assetModel.activate_asset(model.assetId)
                        ToolTip.visible: hovered
                        ToolTip.text: model.path
                    }
                }
                Label { text: "Mesh Preview"; font.bold: true }
                Rectangle {
                    Layout.fillWidth: true; Layout.preferredHeight: 150; color: "#101216"
                    Image { anchors.fill: parent; anchors.margins: 4; fillMode: Image.PreserveAspectFit; source: assetPreviewModel.source }
                    Label { anchors.centerIn: parent; visible: assetPreviewModel.state !== "ready"; text: assetPreviewModel.error !== "" ? assetPreviewModel.error : "Double-click a mesh to preview"; color: "#9aa1ad"; wrapMode: Text.Wrap }
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
                RowLayout {
                    Layout.fillWidth: true
                    ComboBox { id: levelFilter; model: ["", "trace", "debug", "info", "warn", "error"]; onActivated: consoleModel.set_filter(currentText, targetFilter.text) }
                    TextField { id: targetFilter; Layout.fillWidth: true; placeholderText: "Target prefix"; onAccepted: consoleModel.set_filter(levelFilter.currentText, text) }
                    Button { text: "Filter"; onClicked: consoleModel.set_filter(levelFilter.currentText, targetFilter.text) }
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 140
                    model: consoleModel
                    clip: true
                    delegate: Label { width: ListView.view.width; text: model.display; color: "#c4c9d4"; elide: Text.ElideRight }
                }
                Timer { interval: 750; running: editorBridge.status === "Ready"; repeat: true; onTriggered: consoleModel.refresh() }
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
                Canvas {
                    id: profilerChart
                    Layout.fillWidth: true
                    Layout.preferredHeight: 90
                    onPaint: {
                        var ctx = getContext("2d")
                        ctx.fillStyle = "#101216"; ctx.fillRect(0, 0, width, height)
                        var series = profilerModel.series
                        var samples = series.frame
                        var count = samples.length
                        if (!count) return
                        var names = ["frame", "gpu", "draw", "instances", "memory"]
                        var colors = ["#79c0ff", "#d2a8ff", "#f2cc60", "#7ee787", "#ff7b72"]
                        for (var seriesIndex = 0; seriesIndex < names.length; ++seriesIndex) {
                            var values = series[names[seriesIndex]]
                            var maxValue = 1
                            for (var i = 0; i < values.length; ++i) maxValue = Math.max(maxValue, values[i])
                            ctx.strokeStyle = colors[seriesIndex]; ctx.beginPath()
                            for (var j = 0; j < values.length; ++j) {
                                var value = values[j] || 0
                                var x = count === 1 ? 0 : j * width / (count - 1)
                                var y = height - value * height / maxValue
                                if (j === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y)
                            }
                            ctx.stroke()
                        }
                    }
                    Connections { target: profilerModel; function onSeriesChanged() { profilerChart.requestPaint() } }
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 160
                    model: profilerModel
                    clip: true
                    delegate: Label { width: ListView.view.width; text: model.display; color: "#c4c9d4" }
                }
                Label { text: "Asset Graph"; font.bold: true }
                Flickable {
                    id: graphView
                    Layout.fillWidth: true
                    Layout.preferredHeight: 150
                    contentWidth: 800; contentHeight: 500; clip: true
                    Rectangle { width: graphView.contentWidth; height: graphView.contentHeight; color: "#15191f" }
                    Canvas {
                        id: graphLines
                        width: graphView.contentWidth; height: graphView.contentHeight
                        onPaint: {
                            var ctx = getContext("2d"); ctx.clearRect(0, 0, width, height)
                            var edges = assetGraphModel.edgeLines
                            for (var index = 0; index < edges.length; ++index) {
                                var edge = edges[index]; ctx.strokeStyle = edge.highlighted ? "#79c0ff" : "#56606e"; ctx.lineWidth = edge.highlighted ? 2 : 1
                                ctx.beginPath(); ctx.moveTo(edge.x1, edge.y1); ctx.lineTo(edge.x2, edge.y2); ctx.stroke()
                            }
                        }
                        Connections {
                            target: assetGraphModel
                            function onSelectedChanged() { graphLines.requestPaint() }
                            function onModelReset() { graphLines.requestPaint() }
                        }
                    }
                    Repeater {
                        model: assetGraphModel
                        delegate: Rectangle {
                            x: model.x; y: model.y; width: 145; height: 42; radius: 4
                            color: model.highlighted ? "#315a78" : "#252b34"
                            border.color: model.assetId === assetGraphModel.selectedAssetId ? "#79c0ff" : "#4b5563"
                            Label { anchors.fill: parent; anchors.margins: 6; text: model.label; elide: Text.ElideRight; verticalAlignment: Text.AlignVCenter }
                            MouseArea { anchors.fill: parent; onClicked: assetGraphModel.select_asset(model.assetId) }
                        }
                    }
                    PinchHandler { target: null; onActiveScaleChanged: { graphView.contentWidth = Math.max(400, graphView.contentWidth * activeScale); graphView.contentHeight = Math.max(300, graphView.contentHeight * activeScale) } }
                }
            }
        }
    }
}
