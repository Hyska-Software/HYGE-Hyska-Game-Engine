from pathlib import Path


ROOT = Path(__file__).parents[1]


def test_qt_project_lists_runtime_sources_and_qrc() -> None:
    project = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
    assert 'requires-python = ">=3.12,<3.13"' in project
    assert 'PySide6>=6.9,<6.10' in project
    assert '"qml/qml.qrc"' in project
    assert '"src/hyge_editor/main.py"' in project


def test_qml_resource_and_deployment_spec_are_checked_in() -> None:
    qrc = (ROOT / "qml" / "qml.qrc").read_text(encoding="utf-8")
    spec = (ROOT / "pysidedeploy.spec").read_text(encoding="utf-8")
    assert '<file>Main.qml</file>' in qrc
    assert "mode = standalone" in spec
    assert "qml_files = qml/Main.qml" in spec
    assert "project_file = pyproject.toml" in spec


def test_package_launcher_is_relative_and_does_not_use_python() -> None:
    launcher = (ROOT / "HygeEditor.cmd").read_text(encoding="utf-8").lower()
    assert "%~dp0" in launcher
    assert "bin\\hyge-tools.exe" in launcher
    assert "hygeeditor.exe" in launcher
    assert "python" not in launcher


def test_package_launcher_selects_a_project_and_uses_dynamic_port() -> None:
    launcher = (ROOT / "HygeEditor.cmd").read_text(encoding="utf-8").lower()
    assert "select_project.ps1" in launcher
    assert 'set "port=0"' in launcher
    assert "hyge-tools.exe" in launcher


def test_project_selector_and_package_readme_are_present() -> None:
    selector = ROOT / "select_project.ps1"
    readme = ROOT / "README.txt"
    assert selector.exists()
    assert "folderbrowserdialog" in selector.read_text(encoding="utf-8").lower()
    assert "hygeeditor.cmd" in readme.read_text(encoding="utf-8").lower()


def test_frozen_frontend_requires_the_package_launcher() -> None:
    main = (ROOT / "src" / "hyge_editor" / "main.py").read_text(encoding="utf-8")
    assert '"Launch HygeEditor.cmd"' in main
    assert 'if not ("__compiled__" in globals()' in main
