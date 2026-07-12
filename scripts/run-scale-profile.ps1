[CmdletBinding()]
param(
    [Parameter()]
    [string] $ArtifactRoot = 'target/scale-artifacts',
    [Parameter()]
    [switch] $ValidateOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Stop-ExternalCommandProcess {
    param(
        [Parameter(Mandatory)]
        [System.Diagnostics.Process] $CommandProcess
    )

    try {
        if (-not $CommandProcess.HasExited) {
            $CommandProcess.Kill($true)
        }
    } catch [System.InvalidOperationException] {
        if (-not $CommandProcess.HasExited) {
            throw
        }
    }
    if (-not $CommandProcess.HasExited) {
        $CommandProcess.WaitForExit()
    }
}

function Invoke-ExternalCommand {
    param(
        [Parameter(Mandatory)]
        [string] $FilePath,
        [Parameter(Mandatory)]
        [string[]] $Arguments,
        [Parameter(Mandatory)]
        [int] $TimeoutSeconds,
        [Parameter()]
        [DateTimeOffset] $Deadline,
        [Parameter(Mandatory)]
        [string] $StdoutPath,
        [Parameter(Mandatory)]
        [string] $StderrPath,
        [Parameter(Mandatory)]
        [long] $MaxStdoutBytes,
        [Parameter(Mandatory)]
        [long] $MaxStderrBytes
    )

    if ($TimeoutSeconds -lt 1 -or $TimeoutSeconds -gt [int]::MaxValue / 1000) {
        throw "invalid timeout for '$FilePath': $TimeoutSeconds seconds"
    }
    if ($MaxStdoutBytes -lt 1 -or $MaxStderrBytes -lt 1) {
        throw 'command output limits must be positive'
    }

    $stdoutFullPath = [System.IO.Path]::GetFullPath($StdoutPath)
    $stderrFullPath = [System.IO.Path]::GetFullPath($StderrPath)
    $pathComparison = if ($IsWindows) {
        [System.StringComparison]::OrdinalIgnoreCase
    } else {
        [System.StringComparison]::Ordinal
    }
    if ($stdoutFullPath.Equals($stderrFullPath, $pathComparison)) {
        throw 'stdout and stderr artifact paths must be distinct'
    }
    foreach ($outputPath in @($stdoutFullPath, $stderrFullPath)) {
        $outputDirectory = Split-Path -Parent $outputPath
        if (-not (Test-Path -LiteralPath $outputDirectory -PathType Container)) {
            throw "command output directory is missing: $outputDirectory"
        }
    }

    $effectiveTimeoutSeconds = $TimeoutSeconds
    if ($PSBoundParameters.ContainsKey('Deadline')) {
        $remainingSeconds = [int] [Math]::Floor(
            ($Deadline - [DateTimeOffset]::UtcNow).TotalSeconds
        )
        if ($remainingSeconds -lt 1) {
            throw "scale collection deadline expired before '$FilePath'"
        }
        $effectiveTimeoutSeconds = [Math]::Min($TimeoutSeconds, $remainingSeconds)
    }

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.CreateNoWindow = $true
    foreach ($argument in $Arguments) {
        [void] $startInfo.ArgumentList.Add($argument)
    }

    $commandProcess = [System.Diagnostics.Process]::new()
    $commandProcess.StartInfo = $startInfo
    $stdoutFileStream = $null
    $stderrFileStream = $null
    $stdoutTask = $null
    $stderrTask = $null
    $commandStarted = $false
    try {
        $stdoutFileStream = [System.IO.FileStream]::new(
            $stdoutFullPath,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::Write,
            [System.IO.FileShare]::Read,
            65536,
            [System.IO.FileOptions]::Asynchronous
        )
        $stderrFileStream = [System.IO.FileStream]::new(
            $stderrFullPath,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::Write,
            [System.IO.FileShare]::Read,
            65536,
            [System.IO.FileOptions]::Asynchronous
        )
        if (-not $commandProcess.Start()) {
            throw "failed to start command '$FilePath'"
        }
        $commandStarted = $true
        $stdoutTask = $commandProcess.StandardOutput.BaseStream.CopyToAsync($stdoutFileStream)
        $stderrTask = $commandProcess.StandardError.BaseStream.CopyToAsync($stderrFileStream)
        $commandStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        $timedOut = $false
        $terminatedForOutputLimit = $false
        while (-not $commandProcess.WaitForExit(25)) {
            if ($stdoutFileStream.Length -gt $MaxStdoutBytes -or
                $stderrFileStream.Length -gt $MaxStderrBytes) {
                $terminatedForOutputLimit = $true
                Stop-ExternalCommandProcess -CommandProcess $commandProcess
                break
            }
            if ($commandStopwatch.ElapsedMilliseconds -ge
                [long] $effectiveTimeoutSeconds * 1000L) {
                $timedOut = $true
                Stop-ExternalCommandProcess -CommandProcess $commandProcess
                break
            }
        }
        [void] $stdoutTask.GetAwaiter().GetResult()
        [void] $stderrTask.GetAwaiter().GetResult()
        $stdoutFileStream.Flush()
        $stderrFileStream.Flush()

        $stdoutBytes = $stdoutFileStream.Length
        $stderrBytes = $stderrFileStream.Length
        $stdoutLimitExceeded = $stdoutBytes -gt $MaxStdoutBytes
        $stderrLimitExceeded = $stderrBytes -gt $MaxStderrBytes
        if ($stdoutLimitExceeded) {
            $stdoutFileStream.SetLength($MaxStdoutBytes)
        }
        if ($stderrLimitExceeded) {
            $stderrFileStream.SetLength($MaxStderrBytes)
        }
        $stdoutFileStream.Dispose()
        $stdoutFileStream = $null
        $stderrFileStream.Dispose()
        $stderrFileStream = $null

        $stdout = $null
        $stderr = $null
        if (-not $stdoutLimitExceeded -and -not $stderrLimitExceeded) {
            $stdout = [System.IO.File]::ReadAllText(
                $stdoutFullPath,
                [System.Text.Encoding]::UTF8
            )
            $stderr = [System.IO.File]::ReadAllText(
                $stderrFullPath,
                [System.Text.Encoding]::UTF8
            )
        }

        return [pscustomobject]@{
            exit_code = $commandProcess.ExitCode
            timed_out = $timedOut
            timeout_seconds = $effectiveTimeoutSeconds
            terminated_for_output_limit = $terminatedForOutputLimit
            stdout_bytes = $stdoutBytes
            stderr_bytes = $stderrBytes
            stdout_limit_exceeded = $stdoutLimitExceeded
            stderr_limit_exceeded = $stderrLimitExceeded
            stdout = $stdout
            stderr = $stderr
        }
    } finally {
        if ($commandStarted -and -not $commandProcess.HasExited) {
            Stop-ExternalCommandProcess -CommandProcess $commandProcess
        }
        if ($null -ne $stdoutTask -and -not $stdoutTask.IsCompleted) {
            try {
                [void] $stdoutTask.GetAwaiter().GetResult()
            } catch {
                Write-Verbose "stdout copy cleanup also failed: $($_.Exception.Message)"
            }
        }
        if ($null -ne $stderrTask -and -not $stderrTask.IsCompleted) {
            try {
                [void] $stderrTask.GetAwaiter().GetResult()
            } catch {
                Write-Verbose "stderr copy cleanup also failed: $($_.Exception.Message)"
            }
        }
        if ($null -ne $stdoutFileStream) {
            $stdoutFileStream.Dispose()
        }
        if ($null -ne $stderrFileStream) {
            $stderrFileStream.Dispose()
        }
        $commandProcess.Dispose()
    }
}

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory)]
        [string] $FilePath,
        [Parameter(Mandatory)]
        [string[]] $Arguments,
        [Parameter(Mandatory)]
        [int] $TimeoutSeconds,
        [Parameter()]
        [DateTimeOffset] $Deadline,
        [Parameter(Mandatory)]
        [string] $StdoutPath,
        [Parameter(Mandatory)]
        [string] $StderrPath,
        [Parameter(Mandatory)]
        [long] $MaxStdoutBytes,
        [Parameter(Mandatory)]
        [long] $MaxStderrBytes,
        [Parameter(Mandatory)]
        [string] $Description
    )

    $externalCommandParameters = @{
        FilePath = $FilePath
        Arguments = $Arguments
        TimeoutSeconds = $TimeoutSeconds
        StdoutPath = $StdoutPath
        StderrPath = $StderrPath
        MaxStdoutBytes = $MaxStdoutBytes
        MaxStderrBytes = $MaxStderrBytes
    }
    if ($PSBoundParameters.ContainsKey('Deadline')) {
        $externalCommandParameters.Deadline = $Deadline
    }
    $commandResultItems = @(Invoke-ExternalCommand @externalCommandParameters)
    if ($commandResultItems.Count -ne 1) {
        $resultTypes = @($commandResultItems | ForEach-Object { $_.GetType().FullName }) -join ', '
        throw "external command returned unexpected result objects: $resultTypes"
    }
    $commandResult = $commandResultItems[0]
    if ($commandResult.timed_out) {
        throw "$Description timed out after $($commandResult.timeout_seconds) seconds"
    }
    if ($commandResult.stdout_limit_exceeded) {
        throw "$Description exceeded stdout limit of $MaxStdoutBytes bytes"
    }
    if ($commandResult.stderr_limit_exceeded) {
        throw "$Description exceeded stderr limit of $MaxStderrBytes bytes"
    }
    if ($commandResult.exit_code -ne 0) {
        throw "$Description failed with exit code $($commandResult.exit_code)"
    }
    return $commandResult
}

function Convert-ProbeOutput {
    param(
        [Parameter(Mandatory)]
        [string] $Text
    )

    $values = [System.Collections.Generic.Dictionary[string, string]]::new(
        [System.StringComparer]::Ordinal
    )
    foreach ($line in ($Text -split '\r?\n')) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        $separator = $line.IndexOf('=')
        if ($separator -le 0) {
            throw "probe emitted an unstructured line: '$line'"
        }
        $key = $line.Substring(0, $separator)
        $value = $line.Substring($separator + 1)
        if ($key -notmatch '^[a-z][a-z0-9_]*$' -or $value.Length -eq 0) {
            throw "probe emitted an invalid key/value line: '$line'"
        }
        if (-not $values.TryAdd($key, $value)) {
            throw "probe emitted duplicate key '$key'"
        }
    }
    return ,$values
}

function Get-ProbeString {
    param(
        [Parameter(Mandatory)]
        [System.Collections.Generic.Dictionary[string, string]] $Values,
        [Parameter(Mandatory)]
        [string] $Key
    )

    if (-not $Values.ContainsKey($Key)) {
        throw "probe omitted required key '$Key'"
    }
    return [string] $Values[$Key]
}

function Get-ProbeUInt64 {
    param(
        [Parameter(Mandatory)]
        [System.Collections.Generic.Dictionary[string, string]] $Values,
        [Parameter(Mandatory)]
        [string] $Key
    )

    if (-not $Values.ContainsKey($Key)) {
        throw "probe omitted required key '$Key'"
    }
    [UInt64] $number = 0
    if (-not [UInt64]::TryParse(
        $Values[$Key],
        [System.Globalization.NumberStyles]::None,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [ref] $number
    )) {
        throw "probe key '$Key' is not an unsigned integer"
    }
    return [UInt64] $number
}

function Assert-ProbeIdentity {
    param(
        [Parameter(Mandatory)]
        [System.Collections.Generic.Dictionary[string, string]] $Values,
        [Parameter(Mandatory)]
        [string] $ExpectedCaseId,
        [Parameter(Mandatory)]
        [UInt64] $ExpectedScale,
        [Parameter(Mandatory)]
        [string] $ExpectedScaleLabel,
        [Parameter(Mandatory)]
        [string] $RunName
    )

    if ((Get-ProbeString -Values $Values -Key 'schema') -ne 'baozi-scale-probe/v1') {
        throw "probe '$RunName' emitted an unsupported schema"
    }
    if ((Get-ProbeString -Values $Values -Key 'case_id') -ne $ExpectedCaseId) {
        throw "probe '$RunName' emitted an unexpected case ID"
    }
    if ((Get-ProbeUInt64 -Values $Values -Key 'scale') -ne $ExpectedScale) {
        throw "probe '$RunName' emitted an unexpected scale"
    }
    if ((Get-ProbeString -Values $Values -Key 'scale_label') -ne $ExpectedScaleLabel) {
        throw "probe '$RunName' emitted an unexpected scale label"
    }
}

function Assert-ProbeValuesEqual {
    param(
        [Parameter(Mandatory)]
        [System.Collections.Generic.Dictionary[string, string]] $Expected,
        [Parameter(Mandatory)]
        [System.Collections.Generic.Dictionary[string, string]] $Actual,
        [Parameter(Mandatory)]
        [string] $RunName
    )

    if ($Expected.Count -ne $Actual.Count) {
        throw "probe '$RunName' disagreed with the first repetition's key set"
    }
    foreach ($key in $Expected.Keys) {
        if (-not $Actual.ContainsKey($key) -or $Actual[$key] -cne $Expected[$key]) {
            throw "probe '$RunName' disagreed with the first repetition at key '$key'"
        }
    }
}

function New-CriterionCommandArguments {
    param(
        [Parameter(Mandatory)]
        [object] $Policy
    )

    return [string[]] @(
        '+1.95.0', 'bench', '--locked', '-p', 'baozi',
        '--bench', 'import_baseline', '--all-features', '--',
        [string] $Policy.criterion.benchmark_group,
        '--noplot', '--save-baseline', 'scheduled',
        '--warm-up-time', [string] $Policy.criterion.warm_up_seconds,
        '--measurement-time', [string] $Policy.criterion.measurement_seconds,
        '--sample-size', [string] $Policy.criterion.sample_size,
        '--nresamples', [string] $Policy.criterion.resamples
    )
}

function New-ProbeBuildCommandArguments {
    return [string[]] @(
        '+1.95.0', 'bench', '--locked', '-p', 'baozi',
        '--bench', 'import_scale_probe', '--all-features', '--no-run',
        '--message-format=json'
    )
}

function New-TimedProbeCommandArguments {
    param(
        [Parameter(Mandatory)]
        [string] $RssPath,
        [Parameter(Mandatory)]
        [string] $ProbeExecutable,
        [Parameter(Mandatory)]
        [string] $ProbeArgument
    )

    return [string[]] @('-v', '-o', $RssPath, $ProbeExecutable, $ProbeArgument)
}

function Convert-GnuTimeRssBytes {
    param(
        [Parameter(Mandatory)]
        [string] $Text,
        [Parameter(Mandatory)]
        [string] $RunName
    )

    $rssMatches = [regex]::Matches(
        $Text,
        '(?m)^\s*Maximum resident set size \(kbytes\):\s*([0-9]+)\s*$'
    )
    if ($rssMatches.Count -eq 0) {
        throw "GNU time output for '$RunName' omitted maximum RSS"
    }
    if ($rssMatches.Count -ne 1) {
        throw "GNU time output for '$RunName' contained duplicate maximum RSS values"
    }
    $rssMatch = $rssMatches[0]
    $rssKilobytes = [UInt64]::Parse(
        $rssMatch.Groups[1].Value,
        [System.Globalization.CultureInfo]::InvariantCulture
    )
    if ($rssKilobytes -gt [UInt64]::MaxValue / 1024) {
        throw "maximum RSS for '$RunName' overflowed bytes"
    }
    if ($rssKilobytes -eq 0) {
        throw "maximum RSS for '$RunName' must be positive"
    }
    return $rssKilobytes * [UInt64] 1024
}

function Assert-StringSequence {
    param(
        [Parameter(Mandatory)]
        [string[]] $Expected,
        [Parameter(Mandatory)]
        [string[]] $Actual,
        [Parameter(Mandatory)]
        [string] $Description
    )

    if ($Expected.Count -ne $Actual.Count) {
        throw "$Description argument count changed from $($Expected.Count) to $($Actual.Count)"
    }
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        if ($Expected[$index] -cne $Actual[$index]) {
            throw "$Description argument $index changed from '$($Expected[$index])' to '$($Actual[$index])'"
        }
    }
}

function Assert-ThrowsWithMessage {
    param(
        [Parameter(Mandatory)]
        [scriptblock] $Action,
        [Parameter(Mandatory)]
        [string] $ExpectedMessage,
        [Parameter(Mandatory)]
        [string] $Description
    )

    $didThrow = $false
    try {
        & $Action | Out-Null
    } catch {
        $didThrow = $true
        $caughtMessage = [string] $_.Exception.Message
        if (-not $caughtMessage.Contains($ExpectedMessage, [System.StringComparison]::Ordinal)) {
            throw "$Description failed with an unexpected message: $caughtMessage"
        }
    }
    if (-not $didThrow) {
        throw "$Description unexpectedly succeeded"
    }
}

function Assert-JsonSchemaRejects {
    param(
        [Parameter(Mandatory)]
        [string] $Json,
        [Parameter(Mandatory)]
        [string] $SchemaPath,
        [Parameter(Mandatory)]
        [string] $Description
    )

    $rejected = $false
    try {
        $rejected = -not (Test-Json `
            -Json $Json `
            -SchemaFile $SchemaPath `
            -WarningAction SilentlyContinue `
            -ErrorAction Stop)
    } catch {
        $rejected = $true
    }
    if (-not $rejected) {
        throw "$Description unexpectedly satisfied its JSON Schema"
    }
}

function Invoke-TimedProbe {
    param(
        [Parameter(Mandatory)]
        [string] $TimeTool,
        [Parameter(Mandatory)]
        [string] $ProbeExecutable,
        [Parameter(Mandatory)]
        [string] $ProbeArgument,
        [Parameter(Mandatory)]
        [string] $RunName,
        [Parameter(Mandatory)]
        [string] $ProbeDirectory,
        [Parameter(Mandatory)]
        [string] $RssDirectory,
        [Parameter(Mandatory)]
        [int] $TimeoutSeconds,
        [Parameter(Mandatory)]
        [DateTimeOffset] $Deadline,
        [Parameter(Mandatory)]
        [long] $MaxStdoutBytes,
        [Parameter(Mandatory)]
        [long] $MaxStderrBytes
    )

    $probePath = Join-Path $ProbeDirectory "$RunName.txt"
    $probeErrorPath = Join-Path $ProbeDirectory "$RunName.stderr.txt"
    $rssPath = Join-Path $RssDirectory "$RunName.txt"
    $timeArguments = New-TimedProbeCommandArguments `
        -RssPath $rssPath `
        -ProbeExecutable $ProbeExecutable `
        -ProbeArgument $ProbeArgument
    $probeRun = Invoke-CheckedCommand `
        -FilePath $TimeTool `
        -Arguments $timeArguments `
        -TimeoutSeconds $TimeoutSeconds `
        -Deadline $Deadline `
        -StdoutPath $probePath `
        -StderrPath $probeErrorPath `
        -MaxStdoutBytes $MaxStdoutBytes `
        -MaxStderrBytes $MaxStderrBytes `
        -Description "scale probe '$RunName'"
    $rssBytes = Convert-GnuTimeRssBytes `
        -Text (Get-Content -Raw -LiteralPath $rssPath) `
        -RunName $RunName

    [pscustomobject]@{
        rss_bytes = $rssBytes
        values = Convert-ProbeOutput -Text $probeRun.stdout
    }
}

function Test-FinitePositive {
    param([double] $Value)
    return $Value -gt 0.0 -and
        -not [double]::IsNaN($Value) -and
        -not [double]::IsInfinity($Value)
}

function Convert-BoundedEvidenceText {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyString()]
        [string] $Text,
        [Parameter()]
        [int] $MaxUtf8Bytes = 2048
    )

    if ($MaxUtf8Bytes -lt 4) {
        throw 'bounded evidence text requires at least four UTF-8 bytes'
    }
    $normalized = [regex]::Replace($Text, '[\x00-\x1f\x7f]', ' ')
    $maxCharacters = [Math]::Max(1, [int] [Math]::Floor($MaxUtf8Bytes / 4.0))
    $truncated = $normalized.Length -gt $maxCharacters
    if ($truncated) {
        $normalized = $normalized.Substring(0, $maxCharacters)
    }
    return [pscustomobject]@{
        text = $normalized
        truncated = $truncated
    }
}

function Assert-ObjectProperties {
    param(
        [Parameter(Mandatory)]
        [AllowNull()]
        [object] $Value,
        [Parameter(Mandatory)]
        [string[]] $Required,
        [Parameter(Mandatory)]
        [string] $Context
    )

    if ($null -eq $Value) {
        throw "$Context must not be null"
    }
    $propertyNames = @($Value.PSObject.Properties.Name)
    foreach ($requiredProperty in $Required) {
        if ($propertyNames -notcontains $requiredProperty) {
            throw "$Context omitted required property '$requiredProperty'"
        }
    }
}

function Assert-ScaleReportDocument {
    param(
        [Parameter(Mandatory)]
        [object] $Report,
        [Parameter(Mandatory)]
        [ValidateSet('collection', 'validate_only')]
        [string] $ExpectedExecutionMode
    )

    Assert-ObjectProperties -Value $Report -Context 'scale report' -Required @(
        'schema', 'execution_mode', 'status', 'profile_id', 'started_at', 'finished_at',
        'git', 'runner', 'tools', 'policy', 'samples', 'time_slopes', 'checks',
        'advisory_regressions', 'failure'
    )
    if ($Report.schema -cne 'baozi-scale-report/v1') {
        throw "scale report used unsupported schema '$($Report.schema)'"
    }
    if ($Report.execution_mode -cne $ExpectedExecutionMode) {
        throw "scale report execution mode '$($Report.execution_mode)' did not match '$ExpectedExecutionMode'"
    }
    if ($Report.status -notin @('passed', 'failed', 'error')) {
        throw "scale report used unsupported status '$($Report.status)'"
    }
    if ($Report.profile_id -cne 'u3-obj-import-v1') {
        throw "scale report used unsupported profile ID '$($Report.profile_id)'"
    }
    foreach ($timestampProperty in @('started_at', 'finished_at')) {
        [DateTimeOffset] $parsedTimestamp = [DateTimeOffset]::MinValue
        if (-not [DateTimeOffset]::TryParse(
            [string] $Report.$timestampProperty,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [System.Globalization.DateTimeStyles]::RoundtripKind,
            [ref] $parsedTimestamp
        )) {
            throw "scale report property '$timestampProperty' was not an RFC 3339 timestamp"
        }
    }
    Assert-ObjectProperties -Value $Report.git -Context 'scale report git evidence' -Required @(
        'sha', 'ref', 'run_id', 'run_attempt'
    )
    if ($Report.status -eq 'passed') {
        if ($null -ne $Report.failure) {
            throw 'passed scale report must not contain failure evidence'
        }
        Assert-ObjectProperties -Value $Report.runner -Context 'scale report runner evidence' `
            -Required @('os', 'arch', 'image')
        Assert-ObjectProperties -Value $Report.tools -Context 'scale report tool evidence' `
            -Required @('rustc', 'cargo', 'criterion', 'gnu_time')
        Assert-ObjectProperties -Value $Report.policy -Context 'scale report policy evidence' `
            -Required @('schema', 'sha256')
        if ($Report.policy.schema -cne 'baozi-scale-policy/v1' -or
            [string] $Report.policy.sha256 -notmatch '^[0-9a-f]{64}$') {
            throw 'passed scale report contained invalid policy identity evidence'
        }
    } elseif ($Report.status -eq 'error') {
        Assert-ObjectProperties -Value $Report.failure -Context 'scale report failure evidence' `
            -Required @('step', 'exception_type', 'message', 'message_truncated')
    }

    foreach ($check in @($Report.checks)) {
        Assert-ObjectProperties -Value $check -Context 'scale report check' -Required @(
            'id', 'severity', 'passed', 'observed', 'limit', 'unit'
        )
        if ($check.severity -notin @('hard', 'advisory') -or
            $check.passed -isnot [bool]) {
            throw "scale report check '$($check.id)' used an invalid severity or result"
        }
    }
    $checkIds = @($Report.checks | ForEach-Object { [string] $_.id })
    if (@($checkIds | Sort-Object -Unique).Count -ne $checkIds.Count) {
        throw 'scale report contained duplicate check IDs'
    }
    $reportedAdvisoryFailures = @($Report.checks | Where-Object {
        $_.severity -eq 'advisory' -and -not $_.passed
    }).Count
    if ([int] $Report.advisory_regressions -ne $reportedAdvisoryFailures) {
        throw 'scale report advisory regression count disagreed with its checks'
    }
    $reportedHardFailures = @($Report.checks | Where-Object {
        $_.severity -eq 'hard' -and -not $_.passed
    }).Count
    if (($Report.status -eq 'passed' -and $reportedHardFailures -ne 0) -or
        ($Report.status -eq 'failed' -and $reportedHardFailures -eq 0)) {
        throw 'scale report status disagreed with its hard checks'
    }
    foreach ($sample in @($Report.samples)) {
        Assert-ObjectProperties -Value $sample -Context 'scale report sample' -Required @(
            'case_id', 'scale', 'logical_units', 'input_bytes', 'criterion', 'process',
            'ledger'
        )
        Assert-ObjectProperties -Value $sample.criterion -Context 'sample Criterion evidence' `
            -Required @(
                'median_nanoseconds', 'confidence_lower_nanoseconds',
                'confidence_upper_nanoseconds', 'sample_count'
            )
        Assert-ObjectProperties -Value $sample.process -Context 'sample process evidence' `
            -Required @(
                'max_rss_bytes', 'rss_repetitions_bytes', 'idle_probe_min_rss_bytes',
                'idle_probe_max_rss_bytes', 'idle_probe_repetitions_bytes', 'rss_delta_bytes'
            )
        Assert-ObjectProperties -Value $sample.ledger -Context 'sample ledger evidence' `
            -Required @(
                'current_live_bytes', 'peak_live_bytes', 'active_reservations',
                'current_evidence_bytes', 'reviewed_peak_live_envelope_bytes',
                'peak_evidence_bytes', 'canonical_output_committed'
            )
    }
}

function Write-ScaleReportDocument {
    param(
        [Parameter(Mandatory)]
        [object] $Report,
        [Parameter(Mandatory)]
        [string] $Path,
        [Parameter(Mandatory)]
        [string] $SchemaPath,
        [Parameter(Mandatory)]
        [ValidateSet('collection', 'validate_only')]
        [string] $ExpectedExecutionMode
    )

    $reportJson = $Report | ConvertTo-Json -Depth 12
    if (-not (Test-Json -Json $reportJson -SchemaFile $SchemaPath -ErrorAction Stop)) {
        throw 'scale report failed JSON Schema validation'
    }
    $parsedReport = $reportJson | ConvertFrom-Json
    Assert-ScaleReportDocument `
        -Report $parsedReport `
        -ExpectedExecutionMode $ExpectedExecutionMode
    Set-Content -LiteralPath $Path -Value $reportJson -Encoding utf8NoBOM
    return $parsedReport
}

$scriptRoot = Split-Path -Parent $PSCommandPath
$repoRoot = Split-Path -Parent $scriptRoot
$policyPath = Join-Path $repoRoot 'ci/scale-profile-policy.json'
$policySchemaPath = Join-Path $repoRoot 'ci/scale-profile-policy.schema.json'
$reportSchemaPath = Join-Path $repoRoot 'ci/scale-profile-report.schema.json'
$targetRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot 'target'))
$artifactPath = if ([System.IO.Path]::IsPathRooted($ArtifactRoot)) {
    [System.IO.Path]::GetFullPath($ArtifactRoot)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ArtifactRoot))
}
$startedAt = [DateTimeOffset]::UtcNow
$artifactInitialized = $false
$reportPath = $null
$workDirectory = $null
$bootstrapStep = 'artifact_path'
try {
$targetPrefix = $targetRoot.TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
) + [System.IO.Path]::DirectorySeparatorChar
if (-not $artifactPath.StartsWith($targetPrefix, [System.StringComparison]::Ordinal)) {
    throw 'artifact root must be inside the repository target directory'
}
if (Test-Path -LiteralPath $artifactPath) {
    throw "artifact root already exists: $artifactPath"
}
$bootstrapStep = 'artifact_initialization'
New-Item -ItemType Directory -Path $artifactPath | Out-Null
$artifactInitialized = $true
$reportFileName = if ($ValidateOnly) { 'validation.json' } else { 'report.json' }
$reportPath = Join-Path $artifactPath $reportFileName

$bootstrapStep = 'policy_load'
if (-not (Test-Path -LiteralPath $policyPath -PathType Leaf)) {
    throw "scale policy is missing: $policyPath"
}
if (-not (Test-Path -LiteralPath $policySchemaPath -PathType Leaf)) {
    throw "scale policy schema is missing: $policySchemaPath"
}
if (-not (Test-Path -LiteralPath $reportSchemaPath -PathType Leaf)) {
    throw "scale report schema is missing: $reportSchemaPath"
}

$policyJson = Get-Content -Raw -LiteralPath $policyPath
if (-not (Test-Json -Json $policyJson -SchemaFile $policySchemaPath -ErrorAction Stop)) {
    throw 'scale policy failed JSON Schema validation'
}
$policy = $policyJson | ConvertFrom-Json
if ($policy.schema -ne 'baozi-scale-policy/v1') {
    throw "unsupported scale policy schema: $($policy.schema)"
}
if ($policy.artifact_schema -ne 'baozi-scale-report/v1') {
    throw "unsupported scale report schema: $($policy.artifact_schema)"
}
if ($policy.profile_id -ne 'u3-obj-import-v1') {
    throw "unsupported scale profile ID: $($policy.profile_id)"
}
if ($policy.criterion.benchmark_group -ne 'obj_face_import_scale') {
    throw "unsupported Criterion benchmark group: $($policy.criterion.benchmark_group)"
}
if ($policy.criterion.expected_version -ne '0.5.1' -or
    $policy.criterion.expected_source -ne 'registry+https://github.com/rust-lang/crates.io-index') {
    throw 'unsupported Criterion package identity in scale policy'
}
$casePolicies = @($policy.cases)
if ($casePolicies.Count -ne 1) {
    throw 'U3 scale policy must contain exactly one case'
}
$casePolicy = $casePolicies[0]
if ($casePolicy.case_id -ne 'obj_repeated_triangle_faces') {
    throw "unsupported U3 scale case ID: $($casePolicy.case_id)"
}
$expectedScales = @(1, 10, 100, 1000)
$policyScales = @($casePolicy.scales | ForEach-Object { [int] $_ })
if (($policyScales -join ',') -ne ($expectedScales -join ',')) {
    throw 'U3 scale policy must contain exactly 1x, 10x, 100x, and selected 1000x'
}
if (-not [bool] $casePolicy.selected_1000x) {
    throw 'U3 OBJ case must mark 1000x as selected evidence'
}

$toolTimeoutSeconds = [int] $policy.execution.tool_timeout_seconds
$metadataTimeoutSeconds = [int] $policy.execution.metadata_timeout_seconds
$criterionTimeoutSeconds = [int] $policy.execution.criterion_timeout_seconds
$probeBuildTimeoutSeconds = [int] $policy.execution.probe_build_timeout_seconds
$maxStdoutBytes = [long] $policy.execution.max_stdout_bytes
$maxStderrBytes = [long] $policy.execution.max_stderr_bytes
$probeTimeoutSeconds = [int] $policy.rss.probe_timeout_seconds
$collectionTimeoutSeconds = [int] $policy.execution.collection_timeout_seconds
foreach ($timeoutValue in @(
    $toolTimeoutSeconds,
    $metadataTimeoutSeconds,
    $criterionTimeoutSeconds,
    $probeBuildTimeoutSeconds,
    $probeTimeoutSeconds,
    $collectionTimeoutSeconds
)) {
    if ($timeoutValue -lt 1) {
        throw 'scale policy command timeouts must be positive'
    }
}
if ($maxStdoutBytes -lt 1 -or $maxStderrBytes -lt 1) {
    throw 'scale policy command output limits must be positive'
}
$collectionDeadline = $startedAt.AddSeconds($collectionTimeoutSeconds)

$timeTool = [string] $policy.rss.tool

$checks = [System.Collections.Generic.List[object]]::new()
$samples = [System.Collections.Generic.List[object]]::new()
$timeSlopes = [System.Collections.Generic.List[object]]::new()
$hardFailures = @()
$advisoryFailures = @()
$rustcVersion = $null
$cargoVersion = $null
$timeVersion = $null
$criterionActualVersion = $null
$criterionActualSource = $null
$runnerEvidence = $null
$reportWritten = $false
$currentStep = 'initialization'

function Add-ScaleCheck {
    param(
        [Parameter(Mandatory)]
        [string] $Id,
        [Parameter(Mandatory)]
        [ValidateSet('hard', 'advisory')]
        [string] $Severity,
        [Parameter(Mandatory)]
        [bool] $Passed,
        [Parameter(Mandatory)]
        [object] $Observed,
        [Parameter(Mandatory)]
        [object] $Limit,
        [Parameter(Mandatory)]
        [string] $Unit
    )

    $checks.Add([pscustomobject]@{
        id = $Id
        severity = $Severity
        passed = $Passed
        observed = $Observed
        limit = $Limit
        unit = $Unit
    })
}

function New-ScaleReport {
    param(
        [Parameter(Mandatory)]
        [ValidateSet('passed', 'failed', 'error')]
        [string] $Status,
        [Parameter()]
        [object] $Failure
    )

    $executionMode = if ($ValidateOnly) { 'validate_only' } else { 'collection' }
    return [ordered]@{
        schema = [string] $policy.artifact_schema
        execution_mode = $executionMode
        status = $Status
        profile_id = [string] $policy.profile_id
        started_at = $startedAt.ToString('O')
        finished_at = [DateTimeOffset]::UtcNow.ToString('O')
        collection_deadline_at = $collectionDeadline.ToString('O')
        git = [ordered]@{
            sha = [string] $env:GITHUB_SHA
            ref = [string] $env:GITHUB_REF
            run_id = [string] $env:GITHUB_RUN_ID
            run_attempt = [string] $env:GITHUB_RUN_ATTEMPT
        }
        runner = $runnerEvidence
        tools = [ordered]@{
            rustc = $rustcVersion
            cargo = $cargoVersion
            criterion = [ordered]@{
                expected_version = [string] $policy.criterion.expected_version
                actual_version = $criterionActualVersion
                expected_source = [string] $policy.criterion.expected_source
                actual_source = $criterionActualSource
            }
            gnu_time = $timeVersion
        }
        policy = [ordered]@{
            schema = [string] $policy.schema
            sha256 = (Get-FileHash -LiteralPath $policyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        samples = @($samples)
        time_slopes = @($timeSlopes)
        checks = @($checks)
        advisory_regressions = $advisoryFailures.Count
        failure = $Failure
    }
}

if ($ValidateOnly) {
    $bootstrapStep = 'validate_only'
    $currentStep = 'policy_schema_contract'
    $invalidPolicy = $policyJson | ConvertFrom-Json
    $invalidPolicy.cases[0].selected_1000x = 'true'
    $invalidPolicyJson = $invalidPolicy | ConvertTo-Json -Depth 12
    Assert-JsonSchemaRejects `
        -Json $invalidPolicyJson `
        -SchemaPath $policySchemaPath `
        -Description 'string-valued selected_1000x policy'

    $invalidStdoutLimitPolicy = $policyJson | ConvertFrom-Json
    $invalidStdoutLimitPolicy.execution.max_stdout_bytes = 0
    Assert-JsonSchemaRejects `
        -Json ($invalidStdoutLimitPolicy | ConvertTo-Json -Depth 12) `
        -SchemaPath $policySchemaPath `
        -Description 'zero stdout byte limit policy'
    $invalidStderrLimitPolicy = $policyJson | ConvertFrom-Json
    $invalidStderrLimitPolicy.execution.max_stderr_bytes = 0
    Assert-JsonSchemaRejects `
        -Json ($invalidStderrLimitPolicy | ConvertTo-Json -Depth 12) `
        -SchemaPath $policySchemaPath `
        -Description 'zero stderr byte limit policy'

    $currentStep = 'output_limit_contract'
    $outputLimitDirectory = (
        New-Item -ItemType Directory -Path (Join-Path $artifactPath 'output-limit-contract')
    ).FullName
    $currentPowerShellProcess = [System.Diagnostics.Process]::GetCurrentProcess()
    try {
        $currentPowerShellPath = $currentPowerShellProcess.MainModule.FileName
    } finally {
        $currentPowerShellProcess.Dispose()
    }
    if ([string]::IsNullOrWhiteSpace($currentPowerShellPath)) {
        throw 'could not resolve the current PowerShell executable for output limit validation'
    }
    $exactStdoutPath = Join-Path $outputLimitDirectory 'exact.stdout.txt'
    $exactStderrPath = Join-Path $outputLimitDirectory 'exact.stderr.txt'
    $exactOutputCommand =
        "[Console]::Out.Write([string]::new([char]'O', 64)); " +
        "[Console]::Error.Write([string]::new([char]'E', 64))"
    $exactOutputRun = Invoke-CheckedCommand `
        -FilePath $currentPowerShellPath `
        -Arguments @('-NoLogo', '-NoProfile', '-NonInteractive', '-Command', $exactOutputCommand) `
        -TimeoutSeconds 30 `
        -StdoutPath $exactStdoutPath `
        -StderrPath $exactStderrPath `
        -MaxStdoutBytes 64 `
        -MaxStderrBytes 64 `
        -Description 'exact-boundary output command'
    if ($exactOutputRun.stdout_bytes -ne 64 -or
        $exactOutputRun.stderr_bytes -ne 64 -or
        $exactOutputRun.stdout.Length -ne 64 -or
        $exactOutputRun.stderr.Length -ne 64) {
        throw 'an output stream at its exact byte limit was not preserved'
    }

    $stdoutOverflowPath = Join-Path $outputLimitDirectory 'overflow.stdout.txt'
    $stdoutOverflowStderrPath = Join-Path $outputLimitDirectory 'overflow.stdout.stderr.txt'
    $stdoutOverflowCommand =
        "[Console]::Out.Write([string]::new([char]'O', 65536)); " +
        '[Console]::Out.Flush(); Start-Sleep -Seconds 30'
    Assert-ThrowsWithMessage `
        -Action {
            Invoke-CheckedCommand `
                -FilePath $currentPowerShellPath `
                -Arguments @(
                    '-NoLogo', '-NoProfile', '-NonInteractive', '-Command',
                    $stdoutOverflowCommand
                ) `
                -TimeoutSeconds 5 `
                -StdoutPath $stdoutOverflowPath `
                -StderrPath $stdoutOverflowStderrPath `
                -MaxStdoutBytes 64 `
                -MaxStderrBytes 64 `
                -Description 'stdout overflow command'
        } `
        -ExpectedMessage 'exceeded stdout limit of 64 bytes' `
        -Description 'stdout byte limit enforcement'
    if ((Get-Item -LiteralPath $stdoutOverflowPath).Length -ne 64) {
        throw 'stdout overflow artifact was not truncated to its policy limit'
    }

    $stderrOverflowStdoutPath = Join-Path $outputLimitDirectory 'overflow.stderr.stdout.txt'
    $stderrOverflowPath = Join-Path $outputLimitDirectory 'overflow.stderr.txt'
    $stderrOverflowCommand =
        "[Console]::Error.Write([string]::new([char]'E', 65536)); " +
        '[Console]::Error.Flush(); Start-Sleep -Seconds 30'
    Assert-ThrowsWithMessage `
        -Action {
            Invoke-CheckedCommand `
                -FilePath $currentPowerShellPath `
                -Arguments @(
                    '-NoLogo', '-NoProfile', '-NonInteractive', '-Command',
                    $stderrOverflowCommand
                ) `
                -TimeoutSeconds 5 `
                -StdoutPath $stderrOverflowStdoutPath `
                -StderrPath $stderrOverflowPath `
                -MaxStdoutBytes 64 `
                -MaxStderrBytes 64 `
                -Description 'stderr overflow command'
        } `
        -ExpectedMessage 'exceeded stderr limit of 64 bytes' `
        -Description 'stderr byte limit enforcement'
    if ((Get-Item -LiteralPath $stderrOverflowPath).Length -ne 64) {
        throw 'stderr overflow artifact was not truncated to its policy limit'
    }

    $currentStep = 'command_contract'
    $criterionArguments = New-CriterionCommandArguments -Policy $policy
    $expectedCriterionArguments = [string[]] @(
        '+1.95.0', 'bench', '--locked', '-p', 'baozi',
        '--bench', 'import_baseline', '--all-features', '--',
        'obj_face_import_scale', '--noplot', '--save-baseline', 'scheduled',
        '--warm-up-time', '1', '--measurement-time', '3',
        '--sample-size', '20', '--nresamples', '2000'
    )
    Assert-StringSequence `
        -Expected $expectedCriterionArguments `
        -Actual $criterionArguments `
        -Description 'Criterion command'

    $probeBuildArguments = New-ProbeBuildCommandArguments
    $expectedProbeBuildArguments = [string[]] @(
        '+1.95.0', 'bench', '--locked', '-p', 'baozi',
        '--bench', 'import_scale_probe', '--all-features', '--no-run',
        '--message-format=json'
    )
    Assert-StringSequence `
        -Expected $expectedProbeBuildArguments `
        -Actual $probeBuildArguments `
        -Description 'scale probe build command'

    $timedProbeArguments = New-TimedProbeCommandArguments `
        -RssPath '/tmp/baozi scale rss.txt' `
        -ProbeExecutable '/tmp/baozi scale probe' `
        -ProbeArgument '--synthetic-scale=10'
    Assert-StringSequence `
        -Expected @(
            '-v', '-o', '/tmp/baozi scale rss.txt',
            '/tmp/baozi scale probe', '--synthetic-scale=10'
        ) `
        -Actual $timedProbeArguments `
        -Description 'GNU time probe command'

    $currentStep = 'synthetic_probe_contract'
    $syntheticProbeText = @'
schema=baozi-scale-probe/v1
case_id=obj_repeated_triangle_faces
scale=10
scale_label=10x
logical_units=2560
input_bytes=4096
'@
    $syntheticProbeValues = Convert-ProbeOutput -Text $syntheticProbeText
    Assert-ProbeIdentity `
        -Values $syntheticProbeValues `
        -ExpectedCaseId 'obj_repeated_triangle_faces' `
        -ExpectedScale 10UL `
        -ExpectedScaleLabel '10x' `
        -RunName 'validate-only'
    if ((Get-ProbeUInt64 -Values $syntheticProbeValues -Key 'logical_units') -ne 2560UL) {
        throw 'validate-only probe parser changed the logical unit value'
    }
    Assert-ThrowsWithMessage `
        -Action { Convert-ProbeOutput -Text "schema=one`nschema=two" } `
        -ExpectedMessage "duplicate key 'schema'" `
        -Description 'duplicate probe key rejection'
    Assert-ThrowsWithMessage `
        -Action {
            $overflowValues = Convert-ProbeOutput -Text 'logical_units=18446744073709551616'
            Get-ProbeUInt64 -Values $overflowValues -Key 'logical_units'
        } `
        -ExpectedMessage "is not an unsigned integer" `
        -Description 'overflowing probe value rejection'

    $currentStep = 'synthetic_rss_contract'
    $syntheticRssText = @'
Command being timed: "baozi-scale-probe 10"
Maximum resident set size (kbytes): 12345
Exit status: 0
'@
    $syntheticRssBytes = Convert-GnuTimeRssBytes `
        -Text $syntheticRssText `
        -RunName 'validate-only'
    if ($syntheticRssBytes -ne 12641280UL) {
        throw "validate-only RSS parser returned $syntheticRssBytes bytes"
    }
    Assert-ThrowsWithMessage `
        -Action {
            Convert-GnuTimeRssBytes `
                -Text 'Maximum resident set size (kbytes): -1' `
                -RunName 'invalid-rss'
        } `
        -ExpectedMessage 'omitted maximum RSS' `
        -Description 'negative RSS rejection'
    Assert-ThrowsWithMessage `
        -Action {
            Convert-GnuTimeRssBytes `
                -Text "Maximum resident set size (kbytes): 1`nMaximum resident set size (kbytes): 2" `
                -RunName 'duplicate-rss'
        } `
        -ExpectedMessage 'duplicate maximum RSS values' `
        -Description 'duplicate RSS rejection'

    $currentStep = 'synthetic_report_contract'
    $runnerEvidence = [ordered]@{
        os = 'synthetic'
        arch = 'synthetic'
        image = [ordered]@{ os = 'validate-only'; version = '1' }
    }
    $rustcVersion = 'not executed by validate-only'
    $cargoVersion = 'not executed by validate-only'
    $criterionActualVersion = [string] $policy.criterion.expected_version
    $criterionActualSource = [string] $policy.criterion.expected_source
    $timeVersion = 'synthetic GNU time output'
    Add-ScaleCheck `
        -Id 'validate_only_command_contract' `
        -Severity hard `
        -Passed $true `
        -Observed 3 `
        -Limit 3 `
        -Unit 'command_argument_sequences'
    Add-ScaleCheck `
        -Id 'validate_only_probe_and_rss_contract' `
        -Severity hard `
        -Passed $true `
        -Observed $syntheticRssBytes `
        -Limit 12641280UL `
        -Unit 'bytes'
    Add-ScaleCheck `
        -Id 'validate_only_output_limit_contract' `
        -Severity hard `
        -Passed $true `
        -Observed ([ordered]@{ stdout = 64; stderr = 64 }) `
        -Limit ([ordered]@{ stdout = 64; stderr = 64 }) `
        -Unit 'bytes_per_stream'
    $samples.Add([pscustomobject]@{
        case_id = 'obj_repeated_triangle_faces'
        scale = 10
        logical_units = 2560
        input_bytes = 4096
        criterion = [pscustomobject]@{
            median_nanoseconds = 1000.0
            confidence_lower_nanoseconds = 900.0
            confidence_upper_nanoseconds = 1100.0
            sample_count = 20
        }
        process = [pscustomobject]@{
            max_rss_bytes = $syntheticRssBytes
            rss_repetitions_bytes = @($syntheticRssBytes)
            idle_probe_min_rss_bytes = 1048576
            idle_probe_max_rss_bytes = 1048576
            idle_probe_repetitions_bytes = @(1048576)
            rss_delta_bytes = $syntheticRssBytes - 1048576
        }
        ledger = [pscustomobject]@{
            current_live_bytes = 0
            peak_live_bytes = 2097152
            active_reservations = 0
            current_evidence_bytes = 0
            reviewed_peak_live_envelope_bytes = 4194304
            peak_evidence_bytes = 4096
            canonical_output_committed = 65536
        }
    })
    $validationReport = New-ScaleReport -Status passed
    $invalidValidationReport = ($validationReport | ConvertTo-Json -Depth 12) | ConvertFrom-Json
    $invalidValidationReport.status = 'error'
    $invalidValidationReportJson = $invalidValidationReport | ConvertTo-Json -Depth 12
    Assert-JsonSchemaRejects `
        -Json $invalidValidationReportJson `
        -SchemaPath $reportSchemaPath `
        -Description 'error report without failure evidence'
    Assert-ThrowsWithMessage `
        -Action {
            Assert-ScaleReportDocument `
                -Report $invalidValidationReport `
                -ExpectedExecutionMode validate_only
        } `
        -ExpectedMessage 'failure evidence' `
        -Description 'invalid report rejection'
    Write-ScaleReportDocument `
        -Report $validationReport `
        -Path $reportPath `
        -SchemaPath $reportSchemaPath `
        -ExpectedExecutionMode validate_only | Out-Null
    $reportWritten = $true
    Write-Output "scale runner contract validation passed: $reportPath"
    return
}

$bootstrapStep = 'rss_tool_validation'
if (-not $IsLinux) {
    throw 'scheduled RSS profiling requires Linux'
}
if (-not (Test-Path -LiteralPath $timeTool -PathType Leaf)) {
    throw "required RSS tool is missing: $timeTool"
}

$logsDirectory = (New-Item -ItemType Directory -Path (Join-Path $artifactPath 'logs')).FullName
$probeDirectory = (New-Item -ItemType Directory -Path (Join-Path $artifactPath 'probe')).FullName
$rssDirectory = (New-Item -ItemType Directory -Path (Join-Path $artifactPath 'rss')).FullName
$criterionArtifactDirectory = (
    New-Item -ItemType Directory -Path (Join-Path $artifactPath 'criterion')
).FullName
$tempRoot = if ($env:RUNNER_TEMP) {
    [System.IO.Path]::GetFullPath($env:RUNNER_TEMP)
} else {
    [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
}
$workRoot = [System.IO.Path]::GetFullPath((Join-Path $tempRoot 'baozi-scale-profile'))
$artifactPrefix = $artifactPath.TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
) + [System.IO.Path]::DirectorySeparatorChar
if ($workRoot.Equals($artifactPath, [System.StringComparison]::Ordinal) -or
    $workRoot.StartsWith($artifactPrefix, [System.StringComparison]::Ordinal)) {
    throw 'artifact root must not contain the private scale-profile work directory'
}
$workDirectory = [System.IO.Path]::GetFullPath(
    (Join-Path $workRoot ([Guid]::NewGuid().ToString('N')))
)
$criterionWorkDirectory = (
    New-Item -ItemType Directory -Path (Join-Path $workDirectory 'criterion') -Force
).FullName

$previousCriterionHome = [Environment]::GetEnvironmentVariable('CRITERION_HOME', 'Process')
$previousLocale = [Environment]::GetEnvironmentVariable('LC_ALL', 'Process')
$originalLocation = Get-Location
try {
    Set-Location -LiteralPath $repoRoot
    [Environment]::SetEnvironmentVariable('CRITERION_HOME', $criterionWorkDirectory, 'Process')
    [Environment]::SetEnvironmentVariable('LC_ALL', 'C', 'Process')

    $currentStep = 'tool_identity'
    $rustcRun = Invoke-CheckedCommand `
        -FilePath 'rustc' `
        -Arguments @('+1.95.0', '-vV') `
        -TimeoutSeconds $toolTimeoutSeconds `
        -Deadline $collectionDeadline `
        -StdoutPath (Join-Path $logsDirectory 'rustc-version.txt') `
        -StderrPath (Join-Path $logsDirectory 'rustc-version.stderr.txt') `
        -MaxStdoutBytes $maxStdoutBytes `
        -MaxStderrBytes $maxStderrBytes `
        -Description 'rustc version capture'
    $rustcVersion = $rustcRun.stdout.Trim()

    $cargoRun = Invoke-CheckedCommand `
        -FilePath 'cargo' `
        -Arguments @('+1.95.0', '-V') `
        -TimeoutSeconds $toolTimeoutSeconds `
        -Deadline $collectionDeadline `
        -StdoutPath (Join-Path $logsDirectory 'cargo-version.txt') `
        -StderrPath (Join-Path $logsDirectory 'cargo-version.stderr.txt') `
        -MaxStdoutBytes $maxStdoutBytes `
        -MaxStderrBytes $maxStderrBytes `
        -Description 'Cargo version capture'
    $cargoVersion = $cargoRun.stdout.Trim()

    $timeRun = Invoke-CheckedCommand `
        -FilePath $timeTool `
        -Arguments @('--version') `
        -TimeoutSeconds $toolTimeoutSeconds `
        -Deadline $collectionDeadline `
        -StdoutPath (Join-Path $logsDirectory 'gnu-time-version.txt') `
        -StderrPath (Join-Path $logsDirectory 'gnu-time-version.stderr.txt') `
        -MaxStdoutBytes $maxStdoutBytes `
        -MaxStderrBytes $maxStderrBytes `
        -Description 'GNU time version capture'
    $timeVersion = $timeRun.stdout.Trim()

    $unameRun = Invoke-CheckedCommand `
        -FilePath 'uname' `
        -Arguments @('-a') `
        -TimeoutSeconds $toolTimeoutSeconds `
        -Deadline $collectionDeadline `
        -StdoutPath (Join-Path $logsDirectory 'uname.txt') `
        -StderrPath (Join-Path $logsDirectory 'uname.stderr.txt') `
        -MaxStdoutBytes $maxStdoutBytes `
        -MaxStderrBytes $maxStderrBytes `
        -Description 'kernel identity capture'
    $lscpuRun = Invoke-CheckedCommand `
        -FilePath 'lscpu' `
        -Arguments @('--json') `
        -TimeoutSeconds $toolTimeoutSeconds `
        -Deadline $collectionDeadline `
        -StdoutPath (Join-Path $logsDirectory 'lscpu.json') `
        -StderrPath (Join-Path $logsDirectory 'lscpu.stderr.txt') `
        -MaxStdoutBytes $maxStdoutBytes `
        -MaxStderrBytes $maxStderrBytes `
        -Description 'CPU identity capture'
    $lscpuDocument = $lscpuRun.stdout | ConvertFrom-Json
    $runnerEvidence = [ordered]@{
        os = [string] $env:RUNNER_OS
        arch = [string] $env:RUNNER_ARCH
        image = [ordered]@{
            os = [string] $env:ImageOS
            version = [string] $env:ImageVersion
        }
        os_description = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        kernel = $unameRun.stdout.Trim()
        cpu = [ordered]@{
            logical_processors = [Environment]::ProcessorCount
            lscpu = @($lscpuDocument.lscpu)
        }
    }

    $currentStep = 'cargo_metadata'
    $metadataRun = Invoke-CheckedCommand `
        -FilePath 'cargo' `
        -Arguments @('+1.95.0', 'metadata', '--locked', '--format-version', '1') `
        -TimeoutSeconds $metadataTimeoutSeconds `
        -Deadline $collectionDeadline `
        -StdoutPath (Join-Path $logsDirectory 'cargo-metadata.json') `
        -StderrPath (Join-Path $logsDirectory 'cargo-metadata.stderr.txt') `
        -MaxStdoutBytes $maxStdoutBytes `
        -MaxStderrBytes $maxStderrBytes `
        -Description 'locked Cargo metadata resolution'
    $metadata = $metadataRun.stdout | ConvertFrom-Json
    $baoziPackages = @($metadata.packages | Where-Object {
        $_.name -eq 'baozi' -and (@($metadata.workspace_members) -contains $_.id)
    })
    if ($baoziPackages.Count -ne 1) {
        throw "expected one Baozi workspace package, found $($baoziPackages.Count)"
    }
    $baoziPackageId = [string] $baoziPackages[0].id
    $baoziResolveNodes = @($metadata.resolve.nodes | Where-Object { $_.id -eq $baoziPackageId })
    if ($baoziResolveNodes.Count -ne 1) {
        throw "expected one Baozi resolve node, found $($baoziResolveNodes.Count)"
    }
    $criterionEdges = @($baoziResolveNodes[0].deps | Where-Object { $_.name -eq 'criterion' })
    if ($criterionEdges.Count -ne 1 -or
        @($criterionEdges[0].dep_kinds | Where-Object { $_.kind -eq 'dev' }).Count -ne 1) {
        throw 'Baozi must resolve exactly one direct Criterion development dependency'
    }
    $criterionPackages = @($metadata.packages | Where-Object {
        $_.id -eq $criterionEdges[0].pkg
    })
    if ($criterionPackages.Count -ne 1) {
        throw "expected one direct Criterion package, found $($criterionPackages.Count)"
    }
    $criterionActualVersion = [string] $criterionPackages[0].version
    $criterionActualSource = [string] $criterionPackages[0].source
    $criterionVersionMatches = $criterionActualVersion -ceq [string] $policy.criterion.expected_version
    $criterionSourceMatches = $criterionActualSource -ceq [string] $policy.criterion.expected_source
    Add-ScaleCheck `
        -Id 'criterion_resolved_version' `
        -Severity hard `
        -Passed $criterionVersionMatches `
        -Observed $criterionActualVersion `
        -Limit ([string] $policy.criterion.expected_version) `
        -Unit 'semver'
    Add-ScaleCheck `
        -Id 'criterion_resolved_source' `
        -Severity hard `
        -Passed $criterionSourceMatches `
        -Observed $criterionActualSource `
        -Limit ([string] $policy.criterion.expected_source) `
        -Unit 'cargo_source'
    if (-not $criterionVersionMatches -or -not $criterionSourceMatches) {
        throw 'resolved Criterion identity does not match policy'
    }

    $currentStep = 'criterion_benchmark'
    $criterionArguments = New-CriterionCommandArguments -Policy $policy
    Invoke-CheckedCommand `
        -FilePath 'cargo' `
        -Arguments $criterionArguments `
        -TimeoutSeconds $criterionTimeoutSeconds `
        -Deadline $collectionDeadline `
        -StdoutPath (Join-Path $logsDirectory 'criterion.stdout.txt') `
        -StderrPath (Join-Path $logsDirectory 'criterion.stderr.txt') `
        -MaxStdoutBytes $maxStdoutBytes `
        -MaxStderrBytes $maxStderrBytes `
        -Description 'Criterion scale benchmark' | Out-Null

    $currentStep = 'probe_build'
    $probeBuildArguments = New-ProbeBuildCommandArguments
    $probeBuildRun = Invoke-CheckedCommand `
        -FilePath 'cargo' `
        -Arguments $probeBuildArguments `
        -TimeoutSeconds $probeBuildTimeoutSeconds `
        -Deadline $collectionDeadline `
        -StdoutPath (Join-Path $logsDirectory 'probe-build.jsonl') `
        -StderrPath (Join-Path $logsDirectory 'probe-build.stderr.txt') `
        -MaxStdoutBytes $maxStdoutBytes `
        -MaxStderrBytes $maxStderrBytes `
        -Description 'scale probe build'
    $probeExecutables = [System.Collections.Generic.List[string]]::new()
    foreach ($buildLine in ($probeBuildRun.stdout -split '\r?\n')) {
        if (-not $buildLine.StartsWith('{', [System.StringComparison]::Ordinal)) {
            continue
        }
        $buildMessage = $buildLine | ConvertFrom-Json
        if ($buildMessage.reason -eq 'compiler-artifact' -and
            $buildMessage.package_id -eq $baoziPackageId -and
            $buildMessage.target.name -eq 'import_scale_probe' -and
            @($buildMessage.target.kind) -contains 'bench' -and
            $null -ne $buildMessage.executable) {
            $probeExecutables.Add([string] $buildMessage.executable)
        }
    }
    $distinctProbeExecutables = @($probeExecutables | Sort-Object -Unique)
    if ($distinctProbeExecutables.Count -ne 1) {
        throw "expected one scale probe executable, found $($distinctProbeExecutables.Count)"
    }
    $probeExecutable = $distinctProbeExecutables[0]
    if (-not (Test-Path -LiteralPath $probeExecutable -PathType Leaf)) {
        throw "scale probe executable is missing: $probeExecutable"
    }
    $probeExecutable = (Resolve-Path -LiteralPath $probeExecutable).Path

    $rssRepetitions = [int] $policy.rss.repetitions
    if ($rssRepetitions -lt 1) {
        throw 'RSS repetitions must be positive'
    }
    $currentStep = 'idle_probe'
    $baselineMinRss = [UInt64]::MaxValue
    $baselineMaxRss = 0UL
    $baselineRssValues = [System.Collections.Generic.List[UInt64]]::new()
    $baselineProbeValues = $null
    for ($repeat = 1; $repeat -le $rssRepetitions; $repeat++) {
        $baselineRunName = "idle-$repeat"
        $baselineRun = Invoke-TimedProbe `
            -TimeTool $timeTool `
            -ProbeExecutable $probeExecutable `
            -ProbeArgument 'idle' `
            -RunName $baselineRunName `
            -ProbeDirectory $probeDirectory `
            -RssDirectory $rssDirectory `
            -TimeoutSeconds $probeTimeoutSeconds `
            -Deadline $collectionDeadline `
            -MaxStdoutBytes $maxStdoutBytes `
            -MaxStderrBytes $maxStderrBytes
        Assert-ProbeIdentity `
            -Values $baselineRun.values `
            -ExpectedCaseId 'idle_default_registry' `
            -ExpectedScale 0UL `
            -ExpectedScaleLabel 'idle' `
            -RunName $baselineRunName
        if ($null -eq $baselineProbeValues) {
            $baselineProbeValues = $baselineRun.values
        } else {
            Assert-ProbeValuesEqual `
                -Expected $baselineProbeValues `
                -Actual $baselineRun.values `
                -RunName $baselineRunName
        }
        $baselineRunRss = [UInt64] $baselineRun.rss_bytes
        $baselineRssValues.Add($baselineRunRss)
        $baselineMinRss = [Math]::Min($baselineMinRss, $baselineRunRss)
        $baselineMaxRss = [Math]::Max($baselineMaxRss, $baselineRunRss)
    }

    $previousRssDelta = $null
    $previousScale = $null
    $criterionMedians = [System.Collections.Generic.Dictionary[int, double]]::new()
    foreach ($scaleValue in $policyScales) {
        $scaleLabel = "${scaleValue}x"
        $currentStep = "scale_$scaleLabel"
        $estimatePath = Join-Path $criterionWorkDirectory `
            "$($policy.criterion.benchmark_group)/$scaleLabel/new/estimates.json"
        $samplePath = Join-Path $criterionWorkDirectory `
            "$($policy.criterion.benchmark_group)/$scaleLabel/new/sample.json"
        if (-not (Test-Path -LiteralPath $estimatePath -PathType Leaf)) {
            throw "Criterion estimate is missing: $estimatePath"
        }
        if (-not (Test-Path -LiteralPath $samplePath -PathType Leaf)) {
            throw "Criterion sample is missing: $samplePath"
        }
        $criterionScaleArtifactDirectory = (
            New-Item -ItemType Directory -Force -Path (
                Join-Path $criterionArtifactDirectory `
                    "$($policy.criterion.benchmark_group)/$scaleLabel/new"
            )
        ).FullName
        Copy-Item `
            -LiteralPath $estimatePath `
            -Destination (Join-Path $criterionScaleArtifactDirectory 'estimates.json')
        Copy-Item `
            -LiteralPath $samplePath `
            -Destination (Join-Path $criterionScaleArtifactDirectory 'sample.json')
        $estimate = Get-Content -Raw -LiteralPath $estimatePath | ConvertFrom-Json
        $rawSample = Get-Content -Raw -LiteralPath $samplePath | ConvertFrom-Json
        $medianNanoseconds = [double] $estimate.median.point_estimate
        $medianLower = [double] $estimate.median.confidence_interval.lower_bound
        $medianUpper = [double] $estimate.median.confidence_interval.upper_bound
        $iterationSamples = @($rawSample.iters)
        $timeSamples = @($rawSample.times)
        $invalidIterations = @($iterationSamples | Where-Object {
            -not (Test-FinitePositive ([double] $_))
        })
        $invalidTimes = @($timeSamples | Where-Object {
            -not (Test-FinitePositive ([double] $_))
        })
        $criterionComplete = (Test-FinitePositive $medianNanoseconds) -and
            (Test-FinitePositive $medianLower) -and
            (Test-FinitePositive $medianUpper) -and
            $medianLower -le $medianNanoseconds -and
            $medianNanoseconds -le $medianUpper -and
            $iterationSamples.Count -eq [int] $policy.criterion.sample_size -and
            $timeSamples.Count -eq [int] $policy.criterion.sample_size -and
            $iterationSamples.Count -eq $timeSamples.Count -and
            $invalidIterations.Count -eq 0 -and
            $invalidTimes.Count -eq 0
        Add-ScaleCheck `
            -Id "criterion_complete_${scaleLabel}" `
            -Severity hard `
            -Passed $criterionComplete `
            -Observed ([ordered]@{
                iterations = $iterationSamples.Count
                times = $timeSamples.Count
                invalid_iterations = $invalidIterations.Count
                invalid_times = $invalidTimes.Count
            }) `
            -Limit ([int] $policy.criterion.sample_size) `
            -Unit 'samples_per_series'
        if (-not $criterionComplete) {
            throw "Criterion evidence for $scaleLabel is incomplete or non-finite"
        }
        $criterionMedians.Add($scaleValue, $medianNanoseconds)

        $scaleRss = 0UL
        $scaleRssValues = [System.Collections.Generic.List[UInt64]]::new()
        $probeValues = $null
        for ($repeat = 1; $repeat -le $rssRepetitions; $repeat++) {
            $scaleRunName = "$scaleLabel-$repeat"
            $scaleRun = Invoke-TimedProbe `
                -TimeTool $timeTool `
                -ProbeExecutable $probeExecutable `
                -ProbeArgument ([string] $scaleValue) `
                -RunName $scaleRunName `
                -ProbeDirectory $probeDirectory `
                -RssDirectory $rssDirectory `
                -TimeoutSeconds $probeTimeoutSeconds `
                -Deadline $collectionDeadline `
                -MaxStdoutBytes $maxStdoutBytes `
                -MaxStderrBytes $maxStderrBytes
            Assert-ProbeIdentity `
                -Values $scaleRun.values `
                -ExpectedCaseId ([string] $casePolicy.case_id) `
                -ExpectedScale ([UInt64] $scaleValue) `
                -ExpectedScaleLabel $scaleLabel `
                -RunName $scaleRunName
            if ($null -eq $probeValues) {
                $probeValues = $scaleRun.values
            } else {
                Assert-ProbeValuesEqual `
                    -Expected $probeValues `
                    -Actual $scaleRun.values `
                    -RunName $scaleRunName
            }
            $scaleRunRss = [UInt64] $scaleRun.rss_bytes
            $scaleRssValues.Add($scaleRunRss)
            $scaleRss = [Math]::Max($scaleRss, $scaleRunRss)
        }

        $logicalUnits = Get-ProbeUInt64 -Values $probeValues -Key 'logical_units'
        $inputBytes = Get-ProbeUInt64 -Values $probeValues -Key 'input_bytes'
        $ledgerCurrent = Get-ProbeUInt64 -Values $probeValues -Key 'ledger_current_live_bytes'
        $ledgerPeak = Get-ProbeUInt64 -Values $probeValues -Key 'ledger_peak_live_bytes'
        $activeReservations = Get-ProbeUInt64 `
            -Values $probeValues `
            -Key 'ledger_active_reservations'
        $currentEvidence = Get-ProbeUInt64 `
            -Values $probeValues `
            -Key 'ledger_current_evidence_bytes'
        $reviewedPeak = Get-ProbeUInt64 `
            -Values $probeValues `
            -Key 'reviewed_peak_live_envelope_bytes'
        $expectedUnits = [UInt64] $casePolicy.fixture.base_face_count * [UInt64] $scaleValue
        $expectedRecords = $expectedUnits + 4UL
        $expectedTokens = $expectedUnits * 4UL + 14UL
        $expectedIndices = $expectedUnits * 3UL
        $primaryAttempted = Get-ProbeUInt64 -Values $probeValues -Key 'primary_bytes_attempted'
        $primaryConsumed = Get-ProbeUInt64 -Values $probeValues -Key 'primary_bytes_consumed'
        $recordsAttempted = Get-ProbeUInt64 -Values $probeValues -Key 'records_attempted'
        $recordsConsumed = Get-ProbeUInt64 -Values $probeValues -Key 'records_consumed'
        $tokensAttempted = Get-ProbeUInt64 -Values $probeValues -Key 'tokens_attempted'
        $tokensConsumed = Get-ProbeUInt64 -Values $probeValues -Key 'tokens_consumed'
        $facesAttempted = Get-ProbeUInt64 -Values $probeValues -Key 'faces_attempted'
        $facesConsumed = Get-ProbeUInt64 -Values $probeValues -Key 'faces_consumed'
        $policyReviewedPeak = [UInt64] $casePolicy.fixture.peak_fixed_bytes +
            $inputBytes * [UInt64] $casePolicy.fixture.peak_bytes_per_input_byte +
            $logicalUnits * [UInt64] $casePolicy.fixture.peak_bytes_per_face

        Add-ScaleCheck -Id "logical_units_${scaleLabel}" -Severity hard `
            -Passed ($logicalUnits -eq $expectedUnits) -Observed $logicalUnits `
            -Limit $expectedUnits -Unit 'faces'
        Add-ScaleCheck -Id "primary_bytes_${scaleLabel}" -Severity hard `
            -Passed (
                $primaryAttempted -eq $inputBytes -and
                $primaryConsumed -eq $inputBytes
            ) -Observed ([ordered]@{ attempted = $primaryAttempted; consumed = $primaryConsumed }) `
            -Limit $inputBytes -Unit 'bytes'
        Add-ScaleCheck -Id "records_${scaleLabel}" -Severity hard `
            -Passed (
                $recordsAttempted -eq $expectedRecords -and
                $recordsConsumed -eq $expectedRecords
            ) -Observed ([ordered]@{ attempted = $recordsAttempted; consumed = $recordsConsumed }) `
            -Limit $expectedRecords -Unit 'records'
        Add-ScaleCheck -Id "tokens_${scaleLabel}" -Severity hard `
            -Passed (
                $tokensAttempted -eq $expectedTokens -and
                $tokensConsumed -eq $expectedTokens
            ) -Observed ([ordered]@{ attempted = $tokensAttempted; consumed = $tokensConsumed }) `
            -Limit $expectedTokens -Unit 'tokens'
        Add-ScaleCheck -Id "faces_${scaleLabel}" -Severity hard `
            -Passed (
                $facesAttempted -eq $expectedUnits -and
                $facesConsumed -eq $expectedUnits
            ) -Observed ([ordered]@{ attempted = $facesAttempted; consumed = $facesConsumed }) `
            -Limit $expectedUnits -Unit 'faces'
        Add-ScaleCheck -Id "scene_indices_${scaleLabel}" -Severity hard `
            -Passed ((Get-ProbeUInt64 -Values $probeValues -Key 'scene_indices') -eq $expectedIndices) `
            -Observed (Get-ProbeUInt64 -Values $probeValues -Key 'scene_indices') `
            -Limit $expectedIndices -Unit 'indices'
        Add-ScaleCheck -Id "declared_peak_${scaleLabel}" -Severity hard `
            -Passed ($ledgerPeak -le $reviewedPeak) -Observed $ledgerPeak `
            -Limit $reviewedPeak -Unit 'bytes'
        Add-ScaleCheck -Id "reviewed_peak_policy_${scaleLabel}" -Severity hard `
            -Passed ($reviewedPeak -eq $policyReviewedPeak) -Observed $reviewedPeak `
            -Limit $policyReviewedPeak -Unit 'bytes'
        Add-ScaleCheck -Id "terminal_ledger_${scaleLabel}" -Severity hard `
            -Passed (
                $ledgerCurrent -eq 0 -and
                $activeReservations -eq 0 -and
                $currentEvidence -eq 0
            ) -Observed ([ordered]@{
                live_bytes = $ledgerCurrent
                active_reservations = $activeReservations
                evidence_bytes = $currentEvidence
            }) -Limit 0 -Unit 'terminal_zeroes'

        $rssDelta = if ($scaleRss -gt $baselineMinRss) {
            $scaleRss - $baselineMinRss
        } else {
            0UL
        }
        $rssEnvelope = [UInt64] [Math]::Ceiling(
            [double] $inputBytes +
            [double] $policy.rss.ledger_multiplier * [double] $ledgerPeak +
            [double] $policy.rss.fixed_allowance_bytes
        )
        Add-ScaleCheck -Id "rss_absolute_${scaleLabel}" -Severity hard `
            -Passed ($scaleRss -le [UInt64] $policy.rss.max_rss_bytes) `
            -Observed $scaleRss -Limit ([UInt64] $policy.rss.max_rss_bytes) -Unit 'bytes'
        Add-ScaleCheck -Id "rss_ledger_envelope_${scaleLabel}" -Severity hard `
            -Passed ($rssDelta -le $rssEnvelope) -Observed $rssDelta `
            -Limit $rssEnvelope -Unit 'bytes_above_idle_probe'
        if ($null -ne $previousRssDelta) {
            $growthLimit = [UInt64] [Math]::Ceiling(
                [double] $previousRssDelta * [double] $policy.rss.growth_factor_per_decade +
                [double] $policy.rss.growth_slack_bytes
            )
            Add-ScaleCheck -Id "rss_growth_${previousScale}x_to_${scaleLabel}" -Severity hard `
                -Passed ($rssDelta -le $growthLimit) -Observed $rssDelta `
                -Limit $growthLimit -Unit 'bytes_above_idle_probe'
        }

        $samples.Add([pscustomobject]@{
            case_id = [string] $casePolicy.case_id
            scale = $scaleValue
            logical_units = $logicalUnits
            input_bytes = $inputBytes
            criterion = [pscustomobject]@{
                median_nanoseconds = $medianNanoseconds
                confidence_lower_nanoseconds = $medianLower
                confidence_upper_nanoseconds = $medianUpper
                sample_count = $iterationSamples.Count
            }
            process = [pscustomobject]@{
                max_rss_bytes = $scaleRss
                rss_repetitions_bytes = @($scaleRssValues)
                idle_probe_min_rss_bytes = $baselineMinRss
                idle_probe_max_rss_bytes = $baselineMaxRss
                idle_probe_repetitions_bytes = @($baselineRssValues)
                rss_delta_bytes = $rssDelta
            }
            ledger = [pscustomobject]@{
                current_live_bytes = $ledgerCurrent
                peak_live_bytes = $ledgerPeak
                active_reservations = $activeReservations
                current_evidence_bytes = $currentEvidence
                reviewed_peak_live_envelope_bytes = $reviewedPeak
                peak_evidence_bytes = Get-ProbeUInt64 `
                    -Values $probeValues -Key 'ledger_peak_evidence_bytes'
                canonical_output_committed = Get-ProbeUInt64 `
                    -Values $probeValues -Key 'canonical_output_committed'
            }
        })
        $previousRssDelta = $rssDelta
        $previousScale = $scaleValue
    }

    for ($index = 1; $index -lt $policyScales.Count; $index++) {
        $fromScale = $policyScales[$index - 1]
        $toScale = $policyScales[$index]
        $fromMedian = $criterionMedians[$fromScale]
        $toMedian = $criterionMedians[$toScale]
        $exponent = [Math]::Log($toMedian / $fromMedian) /
            [Math]::Log([double] $toScale / [double] $fromScale)
        $slopePassed = $exponent -le [double] $policy.criterion.time_slope_advisory_exponent
        Add-ScaleCheck -Id "criterion_slope_${fromScale}x_to_${toScale}x" `
            -Severity advisory -Passed $slopePassed -Observed $exponent `
            -Limit ([double] $policy.criterion.time_slope_advisory_exponent) -Unit 'power_exponent'
        $timeSlopes.Add([pscustomobject]@{
            from_scale = $fromScale
            to_scale = $toScale
            exponent = $exponent
            advisory_limit = [double] $policy.criterion.time_slope_advisory_exponent
            passed = $slopePassed
        })
    }

    $currentStep = 'artifact_validation'
    $criterionArtifactFiles = @(
        Get-ChildItem -LiteralPath $criterionArtifactDirectory -Recurse -File
    )
    $unexpectedCriterionArtifacts = @($criterionArtifactFiles | Where-Object {
        $_.Name -notin @('estimates.json', 'sample.json')
    })
    $criterionRawOnly =
        $criterionArtifactFiles.Count -eq $policyScales.Count * 2 -and
        $unexpectedCriterionArtifacts.Count -eq 0
    Add-ScaleCheck `
        -Id 'criterion_artifact_raw_only' `
        -Severity hard `
        -Passed $criterionRawOnly `
        -Observed $criterionArtifactFiles.Count `
        -Limit ($policyScales.Count * 2) `
        -Unit 'raw_json_files'

    $htmlArtifacts = @(Get-ChildItem -LiteralPath $artifactPath -Recurse -File -Filter '*.html')
    Add-ScaleCheck `
        -Id 'artifact_html_absent' `
        -Severity hard `
        -Passed ($htmlArtifacts.Count -eq 0) `
        -Observed $htmlArtifacts.Count `
        -Limit 0 `
        -Unit 'html_files'

    $hardFailures = @($checks | Where-Object { $_.severity -eq 'hard' -and -not $_.passed })
    $advisoryFailures = @($checks | Where-Object { $_.severity -eq 'advisory' -and -not $_.passed })
    $reportStatus = if ($hardFailures.Count -eq 0) { 'passed' } else { 'failed' }
    $report = New-ScaleReport -Status $reportStatus
    Write-ScaleReportDocument `
        -Report $report `
        -Path $reportPath `
        -SchemaPath $reportSchemaPath `
        -ExpectedExecutionMode collection | Out-Null
    $reportWritten = $true

    if ($hardFailures.Count -ne 0) {
        $failedIds = $hardFailures.id -join ', '
        throw "scale profile hard checks failed: $failedIds"
    }
    if ($advisoryFailures.Count -ne 0) {
        Write-Warning "Criterion trend advisories failed: $($advisoryFailures.id -join ', ')"
    }
} catch {
    $runFailure = $_
    if (-not $reportWritten) {
        $hardFailures = @($checks | Where-Object { $_.severity -eq 'hard' -and -not $_.passed })
        $advisoryFailures = @($checks | Where-Object {
            $_.severity -eq 'advisory' -and -not $_.passed
        })
        $boundedFailureMessage = Convert-BoundedEvidenceText `
            -Text ([string] $runFailure.Exception.Message)
        $failureEvidence = [ordered]@{
            step = $currentStep
            exception_type = $runFailure.Exception.GetType().FullName
            message = $boundedFailureMessage.text
            message_truncated = $boundedFailureMessage.truncated
        }
        try {
            $failureReport = New-ScaleReport -Status error -Failure $failureEvidence
            Write-ScaleReportDocument `
                -Report $failureReport `
                -Path $reportPath `
                -SchemaPath $reportSchemaPath `
                -ExpectedExecutionMode collection | Out-Null
            $reportWritten = $true
        } catch {
            Write-Warning "failed to write scale error report: $($_.Exception.Message)"
        }
    }
    throw $runFailure
} finally {
    Set-Location -LiteralPath $originalLocation
    [Environment]::SetEnvironmentVariable('CRITERION_HOME', $previousCriterionHome, 'Process')
    [Environment]::SetEnvironmentVariable('LC_ALL', $previousLocale, 'Process')
}
} catch {
    $bootstrapFailure = $_
    $expectedExecutionMode = if ($ValidateOnly) { 'validate_only' } else { 'collection' }
    $existingReportIsValid = $false
    if ($artifactInitialized -and
        -not [string]::IsNullOrWhiteSpace($reportPath) -and
        (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        try {
            $existingReportJson = Get-Content -Raw -LiteralPath $reportPath
            $existingReportMatchesSchema =
                (Test-Path -LiteralPath $reportSchemaPath -PathType Leaf) -and
                (Test-Json `
                    -Json $existingReportJson `
                    -SchemaFile $reportSchemaPath `
                    -ErrorAction Stop)
            $existingReport = $existingReportJson | ConvertFrom-Json
            Assert-ScaleReportDocument `
                -Report $existingReport `
                -ExpectedExecutionMode $expectedExecutionMode
            $existingReportIsValid = $existingReportMatchesSchema -and
                $existingReport.schema -eq 'baozi-scale-report/v1' -and
                $existingReport.execution_mode -eq $expectedExecutionMode -and
                $existingReport.status -in @('failed', 'error')
        } catch {
            $existingReportIsValid = $false
        }
    }
    if ($artifactInitialized -and
        -not [string]::IsNullOrWhiteSpace($reportPath) -and
        -not $existingReportIsValid) {
        $boundedBootstrapMessage = Convert-BoundedEvidenceText `
            -Text ([string] $bootstrapFailure.Exception.Message)
        $bootstrapReport = [ordered]@{
            schema = 'baozi-scale-report/v1'
            execution_mode = $expectedExecutionMode
            status = 'error'
            profile_id = 'u3-obj-import-v1'
            started_at = $startedAt.ToString('O')
            finished_at = [DateTimeOffset]::UtcNow.ToString('O')
            git = [ordered]@{
                sha = [string] $env:GITHUB_SHA
                ref = [string] $env:GITHUB_REF
                run_id = [string] $env:GITHUB_RUN_ID
                run_attempt = [string] $env:GITHUB_RUN_ATTEMPT
            }
            runner = $null
            tools = $null
            policy = $null
            samples = @()
            time_slopes = @()
            checks = @()
            advisory_regressions = 0
            failure = [ordered]@{
                step = $bootstrapStep
                exception_type = $bootstrapFailure.Exception.GetType().FullName
                message = $boundedBootstrapMessage.text
                message_truncated = $boundedBootstrapMessage.truncated
            }
        }
        try {
            $bootstrapReport | ConvertTo-Json -Depth 12 |
                Set-Content -LiteralPath $reportPath -Encoding utf8NoBOM
        } catch {
            Write-Warning "failed to write scale bootstrap report: $($_.Exception.Message)"
        }
    }
    throw $bootstrapFailure
}
