import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ApplicationWindow {
    id: window
    visible: true
    width: editorPreferences.windowWidth
    height: editorPreferences.windowHeight
    title: "Hyge Editor"
    color: editorTheme.window

    property bool textEditorFocused: activeFocusItem && activeFocusItem.objectName === "editorTextField"
    onWidthChanged: if (visible) editorPreferences.set_window_size(width, height)
    onHeightChanged: if (visible) editorPreferences.set_window_size(width, height)
    onActiveChanged: if (!active) viewportInput.clear_transient()

    Shortcut { sequence: editorPreferences.shortcut("save"); enabled: !window.textEditorFocused; onActivated: editorInteraction.save_scene() }
    Shortcut { sequence: editorPreferences.shortcut("undo"); enabled: !window.textEditorFocused; onActivated: editorInteraction.undo() }
    Shortcut { sequence: editorPreferences.shortcut("redo"); enabled: !window.textEditorFocused; onActivated: editorInteraction.redo() }
    Shortcut { sequence: editorPreferences.shortcut("focus_selection"); enabled: !window.textEditorFocused; onActivated: viewportInput.camera("focus", 0, 0) }
    Shortcut { sequence: editorPreferences.shortcut("frame_selection"); enabled: !window.textEditorFocused; onActivated: viewportInput.camera("focus", 1, 0) }
    Shortcut { sequence: editorPreferences.shortcut("toggle_play_editor"); enabled: !window.textEditorFocused; onActivated: editorPreferences.toggle_mode() }

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
            Label { text: "HYGE"; color: editorTheme.text; font.bold: true; font.pixelSize: 18 }
            Label { text: editorBridge.status; color: editorBridge.status === "Ready" ? editorTheme.ok : editorTheme.muted }
            Label { text: editorPreferences.mode === "play" ? "PLAY" : "EDITOR"; color: editorTheme.accent; font.bold: true }
            Item { Layout.fillWidth: true }
            Label { text: editorInteraction.hasConflict ? editorInteraction.conflictMessage : "Revision " + editorInteraction.revision; color: editorInteraction.hasConflict ? editorTheme.error : editorTheme.muted }
            Label { visible: editorInteraction.hasSceneReloadConflict; text: "External scene change"; color: editorTheme.error }
            Label { text: "Dropped: " + editorBridge.droppedFrames; color: editorTheme.muted }
            Button { text: "Connect"; onClicked: editorBridge.connect_backend() }
            Button { text: "Open Project"; onClicked: editorBridge.open_project() }
            Button { text: "Refresh"; onClicked: editorInteraction.refresh_snapshot() }
            Button { text: editorPreferences.theme === "dark" ? "Light" : "Dark"; onClicked: { editorPreferences.toggle_theme(); editorTheme.apply() } }
            Button { text: "Save"; onClicked: editorInteraction.save_scene() }
            Button {
                text: "Panels"
                onClicked: panelMenu.open()
                Menu {
                    id: panelMenu
                    MenuItem { text: "Hierarchy"; checkable: true; checked: editorPreferences.panel_visible("hierarchy"); onTriggered: editorPreferences.set_panel_visible("hierarchy", checked) }
                    MenuItem { text: "Assets"; checkable: true; checked: editorPreferences.panel_visible("assets"); onTriggered: editorPreferences.set_panel_visible("assets", checked) }
                    MenuItem { text: "Inspector"; checkable: true; checked: editorPreferences.panel_visible("inspector"); onTriggered: editorPreferences.set_panel_visible("inspector", checked) }
                    MenuItem { text: "Console"; checkable: true; checked: editorPreferences.panel_visible("console"); onTriggered: editorPreferences.set_panel_visible("console", checked) }
                    MenuItem { text: "Profiler"; checkable: true; checked: editorPreferences.panel_visible("profiler"); onTriggered: editorPreferences.set_panel_visible("profiler", checked) }
                    MenuItem { text: "Asset Graph"; checkable: true; checked: editorPreferences.panel_visible("asset_graph"); onTriggered: editorPreferences.set_panel_visible("asset_graph", checked) }
                }
            }
        }
    }

    Rectangle {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: editorBridge.status === "Ready" ? 0 : 34
        visible: height > 0
        color: editorBridge.status === "Failed" ? editorTheme.error : editorTheme.surfaceAlt
        z: 10
        RowLayout {
            anchors.fill: parent
            anchors.margins: 6
            Label {
                Layout.fillWidth: true
                color: editorTheme.text
                text: editorBridge.status === "Reconnecting"
                    ? "Backend disconnected — retry " + editorBridge.retryAttempt + " in " + editorBridge.retryDelayMs + " ms"
                    : editorBridge.status === "Launch HygeEditor.cmd"
                        ? "Open HygeEditor.cmd to select a project and start the backend"
                    : "Backend: " + editorBridge.status
            }
            Button { text: "Retry now"; visible: editorBridge.status !== "Closing" && editorBridge.status !== "Launch HygeEditor.cmd"; onClicked: editorBridge.retry_backend() }
            Button { text: "Close"; onClicked: editorBridge.close_backend() }
        }
    }

    Dialog {
        id: sceneReloadDialog
        modal: true
        title: "Scene changed on disk"
        standardButtons: Dialog.NoButton
        visible: editorInteraction.hasSceneReloadConflict
        contentItem: ColumnLayout {
            Label { text: "The scene changed outside the editor:\n" + editorInteraction.sceneReloadPath; wrapMode: Text.WordWrap }
            RowLayout {
                Button { text: "Reload"; onClicked: { editorInteraction.resolve_scene_reload("reload_discard"); sceneReloadDialog.close() } }
                Button { text: "Keep editor"; onClicked: { editorInteraction.resolve_scene_reload("keep_editor"); sceneReloadDialog.close() } }
                Button { text: "Save then reload"; onClicked: { editorInteraction.resolve_scene_reload("save_then_reload"); sceneReloadDialog.close() } }
            }
        }
    }

    SplitView {
        anchors.fill: parent

        Frame {
            SplitView.preferredWidth: 330
            SplitView.onPreferredWidthChanged: editorPreferences.set_left_width(SplitView.preferredWidth)
            visible: editorPreferences.panel_visible("hierarchy") || editorPreferences.panel_visible("assets")
            ColumnLayout {
                anchors.fill: parent
                Label { text: "Hierarchy"; color: editorTheme.text; font.bold: true }
                TreeView {
                    id: hierarchyTree
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: hierarchyModel
                    clip: true
                    visible: editorPreferences.panel_visible("hierarchy")
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
                Label { text: "Assets"; color: editorTheme.text; font.bold: true }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 160
                    model: assetModel
                    clip: true
                    visible: editorPreferences.panel_visible("assets")
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
                Label { text: "Mesh Preview"; color: editorTheme.text; font.bold: true }
                Rectangle {
                    Layout.fillWidth: true; Layout.preferredHeight: 150; color: editorTheme.surfaceAlt
                    visible: editorPreferences.panel_visible("assets")
                    Image { anchors.fill: parent; anchors.margins: 4; fillMode: Image.PreserveAspectFit; source: assetPreviewModel.source }
                    Label { anchors.centerIn: parent; visible: assetPreviewModel.state !== "ready"; text: assetPreviewModel.error !== "" ? assetPreviewModel.error : "Double-click a mesh to preview"; color: editorTheme.muted; wrapMode: Text.Wrap }
                }
            }
        }

        Frame {
            SplitView.fillWidth: true
            SplitView.fillHeight: true
            ColumnLayout {
                anchors.fill: parent
                Label { text: "Viewport"; color: editorTheme.text; font.bold: true }
                FocusScope {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    focus: true
                    Keys.onPressed: { if (!event.isAutoRepeat) viewportInput.key(event.key.toString(), true); event.accepted = true }
                    Keys.onReleased: { if (!event.isAutoRepeat) viewportInput.key(event.key.toString(), false); event.accepted = true }
                    onActiveFocusChanged: if (!activeFocus) viewportInput.clear_transient()
                    Rectangle {
                        anchors.fill: parent
                        color: editorTheme.surfaceAlt
                        Image {
                            anchors.fill: parent
                            fillMode: Image.PreserveAspectFit
                            source: "image://hyge-viewport/frame?" + viewportRevision
                            asynchronous: true
                        }
                        MouseArea {
                            id: viewportMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            property real lastX: 0
                            property real lastY: 0
                            onPressed: {
                                parent.parent.forceActiveFocus()
                                lastX = mouse.x; lastY = mouse.y
                                viewportInput.button(mouse.button.toString(), true)
                            }
                            onReleased: viewportInput.button(mouse.button.toString(), false)
                            onPositionChanged: {
                                if (pressed) {
                                    viewportInput.mouse(mouse.x - lastX, mouse.y - lastY)
                                    lastX = mouse.x; lastY = mouse.y
                                }
                            }
                            onWheel: viewportInput.wheel(wheel.angleDelta.x, wheel.angleDelta.y)
                        }
                    }
                    Label {
                        anchors.centerIn: parent
                        visible: viewportController.state !== "connected"
                        text: "Waiting for viewport frame"
                        color: editorTheme.muted
                    }
                }
                Label { text: "Console"; color: editorTheme.text; font.bold: true }
                RowLayout {
                    Layout.fillWidth: true
                    visible: editorPreferences.panel_visible("console")
                    ComboBox { id: levelFilter; model: ["", "trace", "debug", "info", "warn", "error"]; onActivated: consoleModel.set_filter(currentText, targetFilter.text) }
                    TextField { id: targetFilter; Layout.fillWidth: true; placeholderText: "Target prefix"; onAccepted: consoleModel.set_filter(levelFilter.currentText, text) }
                    Button { text: "Filter"; onClicked: consoleModel.set_filter(levelFilter.currentText, targetFilter.text) }
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 140
                    model: consoleModel
                    clip: true
                    visible: editorPreferences.panel_visible("console")
                    delegate: Label { width: ListView.view.width; text: model.display; color: editorTheme.text; elide: Text.ElideRight }
                }
                Timer { interval: 750; running: editorBridge.status === "Ready"; repeat: true; onTriggered: consoleModel.refresh() }
            }
        }

        Frame {
            SplitView.preferredWidth: 390
            SplitView.onPreferredWidthChanged: editorPreferences.set_right_width(SplitView.preferredWidth)
            visible: editorPreferences.panel_visible("inspector") || editorPreferences.panel_visible("profiler") || editorPreferences.panel_visible("asset_graph")
            ColumnLayout {
                anchors.fill: parent
                Label { text: "Inspector"; color: editorTheme.text; font.bold: true }
                TreeView {
                    id: inspectorTree
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    model: inspectorModel
                    clip: true
                    visible: editorPreferences.panel_visible("inspector")
                    delegate: ColumnLayout {
                        required property int depth
                        required property bool isTreeNode
                        width: inspectorTree.width
                        spacing: 2
                        RowLayout {
                            Layout.fillWidth: true
                            Layout.leftMargin: 8 + depth * 14
                            Label { text: model.label; color: editorTheme.text; font.bold: model.nodeKind === "component"; Layout.preferredWidth: 120; elide: Text.ElideRight }
                            Label { text: model.mixed ? "Multiple Values" : model.displayValue; color: model.error ? editorTheme.error : editorTheme.text; Layout.fillWidth: true; elide: Text.ElideRight }
                            TextField {
                                visible: model.editable && model.editorKind !== "group"
                                Layout.preferredWidth: 150
                                objectName: "editorTextField"
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
                        Label { visible: model.error !== ""; text: model.error; color: editorTheme.error; wrapMode: Text.Wrap; Layout.leftMargin: 8 + depth * 14 }
                    }
                }
                Label { text: "Profiler"; color: editorTheme.text; font.bold: true }
                Canvas {
                    id: profilerChart
                    Layout.fillWidth: true
                    Layout.preferredHeight: 90
                    visible: editorPreferences.panel_visible("profiler")
                    onPaint: {
                        var ctx = getContext("2d")
                        ctx.fillStyle = editorTheme.surfaceAlt; ctx.fillRect(0, 0, width, height)
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
                    visible: editorPreferences.panel_visible("profiler")
                    delegate: Label { width: ListView.view.width; text: model.display; color: editorTheme.text }
                }
                Label { text: "Asset Graph"; color: editorTheme.text; font.bold: true }
                Flickable {
                    id: graphView
                    Layout.fillWidth: true
                    Layout.preferredHeight: 150
                    contentWidth: 800; contentHeight: 500; clip: true
                    visible: editorPreferences.panel_visible("asset_graph")
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
