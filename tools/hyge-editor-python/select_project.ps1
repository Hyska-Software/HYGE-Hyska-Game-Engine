Add-Type -AssemblyName System.Windows.Forms

$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = "Select a Hyge project directory"
$dialog.UseDescriptionForTitle = $true

if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::WriteLine($dialog.SelectedPath)
}
