[app]
title = HygeEditor
project_dir = .
input_file = main.py
project_file = pyproject.toml
exec_directory = ../../target/editor-windows
[python]
python_path = .venv-r104/Scripts/python.exe
packages = nuitka,ordered_set,zstandard,PySide6,blake3

[qt]
qml_files = qml/Main.qml
modules = Core,Gui,Network,OpenGL,Qml,QmlMeta,QmlModels,QmlWorkerScript,Quick,QuickControls2,QuickTemplates2,QuickLayouts,Widgets
plugins = platforms,imageformats

[nuitka]
mode = standalone
extra_args = --assume-yes-for-downloads --output-filename=HygeEditor.exe --include-package=qml --include-module=qml.rc_qml --include-data-dir=.venv-r104/Lib/site-packages/PySide6/qml/QtQuick=PySide6/qml/QtQuick

