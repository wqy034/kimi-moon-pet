# 只复制运行必需文件，禁止直接复制整个config或素材目录。
param([switch]$SkipBuild, [string]$VcRuntimeDirectory)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Push-Location $taskRoot
try {
    # ONNX Runtime静态库仍依赖MSVC C++运行库；只取官方Redist/x64目录，不取System32或debug_nonredist。
    $taskCrtDirectory = $VcRuntimeDirectory
    if (-not $taskCrtDirectory) {
        $taskRedistRoots = @()
        if ($env:VCToolsRedistDir) { $taskRedistRoots += $env:VCToolsRedistDir }
        $taskVsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        if (Test-Path -LiteralPath $taskVsWhere) {
            $taskVsInstall = & $taskVsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
            if ($LASTEXITCODE -eq 0 -and $taskVsInstall) {
                $taskRedistBase = Join-Path ($taskVsInstall | Select-Object -First 1) 'VC/Redist/MSVC'
                $taskRedistRoots += @(Get-ChildItem -LiteralPath $taskRedistBase -Directory | Sort-Object Name -Descending | Select-Object -ExpandProperty FullName)
            }
        }
        foreach ($taskRedistRoot in $taskRedistRoots) {
            $taskCandidate = Join-Path $taskRedistRoot 'x64/Microsoft.VC143.CRT'
            if (Test-Path -LiteralPath $taskCandidate -PathType Container) { $taskCrtDirectory = $taskCandidate; break }
        }
    }
    if (-not $taskCrtDirectory -or $taskCrtDirectory -match 'debug_nonredist') {
        throw '找不到MSVC x64可再分发运行库；请安装VS C++工具或传入-VcRuntimeDirectory指定正式x64 CRT目录'
    }
    $taskCrtDirectory = (Resolve-Path -LiteralPath $taskCrtDirectory).Path
    $taskCrtRequired = @('msvcp140.dll','msvcp140_1.dll','vcruntime140.dll','vcruntime140_1.dll')
    $taskCrtOptional = @('concrt140.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll','msvcp140_codecvt_ids.dll','vccorlib140.dll','vcruntime140_threads.dll')
    foreach ($taskCrtName in $taskCrtRequired) {
        if (-not (Test-Path -LiteralPath (Join-Path $taskCrtDirectory $taskCrtName) -PathType Leaf)) { throw "运行库缺少$taskCrtName" }
    }
    if (-not $SkipBuild) {
        & node (Join-Path $taskRoot 'node_modules/@tauri-apps/cli/tauri.js') build --no-bundle
        if ($LASTEXITCODE -ne 0) { throw 'Release构建失败' }
    }
    $taskRev = (& git rev-parse --short HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw '无法读取Git版本' }
    $taskDirty = [bool](& git status --porcelain --untracked-files=normal)
    $taskBuild = $taskRev + $(if ($taskDirty) { '-dirty' } else { '' })
    $taskVersion = (Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json).version
    $taskSchemaText = Get-Content -LiteralPath 'src-tauri/src/chat_store.rs' -Raw -Encoding UTF8
    if ($taskSchemaText -notmatch 'SCHEMA_VERSION:\s*i64\s*=\s*(\d+)') { throw '无法读取聊天数据库版本' }
    $taskSchema = $Matches[1]
    $taskName = 'Moon·月-v' + $taskVersion + '-试玩版-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + $taskBuild
    $taskDist = Join-Path $taskRoot 'dist'
    $taskOut = Join-Path $taskDist $taskName
    if (Test-Path -LiteralPath $taskOut) { throw "输出目录已存在，请保留旧包或换名后重试：$taskOut" }
    New-Item -ItemType Directory -Path $taskOut | Out-Null
    Copy-Item -LiteralPath 'src-tauri/target/release/pet-engine.exe' -Destination (Join-Path $taskOut 'Moon·月.exe')
    foreach ($taskCrtName in ($taskCrtRequired + $taskCrtOptional)) {
        $taskCrtSource = Join-Path $taskCrtDirectory $taskCrtName
        if (Test-Path -LiteralPath $taskCrtSource -PathType Leaf) { Copy-Item -LiteralPath $taskCrtSource -Destination $taskOut }
    }
    $taskCrtVersion = (Get-Item -LiteralPath (Join-Path $taskCrtDirectory 'msvcp140.dll')).VersionInfo.FileVersion
    $taskCrtNotice = @"
Microsoft Visual C++ x64 Runtime
Copyright (c) Microsoft Corporation. All rights reserved.
版本：$taskCrtVersion
来源：Visual Studio官方Redist目录中的x64 Microsoft.VC143.CRT，随应用本地部署。
分发说明：https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files
运行库更新随新版便携包提供。本包不安装或替换Windows系统文件。
DirectML、Direct3D等Windows组件不从开发机复制；需要Windows 10 1903及以上的x64系统或Windows 11。
"@
    [IO.File]::WriteAllText((Join-Path $taskOut '运行库说明.txt'), $taskCrtNotice, [Text.UTF8Encoding]::new($true))
    New-Item -ItemType Directory -Path (Join-Path $taskOut 'config'),(Join-Path $taskOut '道具') | Out-Null
    foreach ($taskConfig in @('动画.json','可见边界.json','养成.json','交流人设.json','状态映射.json')) {
        Copy-Item -LiteralPath (Join-Path 'config' $taskConfig) -Destination (Join-Path $taskOut 'config')
    }
    New-Item -ItemType Directory -Path (Join-Path $taskOut 'config/曲库') | Out-Null
    Get-ChildItem -LiteralPath 'config/曲库' -File -Filter '*.json' | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $taskOut 'config/曲库')
    }
    New-Item -ItemType Directory -Path (Join-Path $taskOut '音源署名') | Out-Null
    foreach ($taskCredit in @('ATTRIBUTION.txt','SOURCE-README.txt')) {
        Copy-Item -LiteralPath (Join-Path 'app/audio/piano' $taskCredit) -Destination (Join-Path $taskOut '音源署名')
    }
    $taskActions = (Get-Content -LiteralPath 'config/动画.json' -Raw -Encoding UTF8 | ConvertFrom-Json).actions
    $taskFrames = @{}
    foreach ($taskAction in $taskActions.PSObject.Properties.Value) {
        foreach ($taskFrame in $taskAction.frames) {
            $taskRel = '帧文件/' + $taskAction.dir + '/' + $taskFrame + '.png'
            $taskFrames[$taskRel] = $true
        }
    }
    foreach ($taskRel in $taskFrames.Keys) {
        $taskTarget = Join-Path $taskOut $taskRel
        New-Item -ItemType Directory -Path (Split-Path $taskTarget -Parent) -Force | Out-Null
        Copy-Item -LiteralPath $taskRel -Destination $taskTarget
    }
    foreach ($taskProp in @('睡觉_Zzz.png','鼠标指针.png')) {
        Copy-Item -LiteralPath (Join-Path '道具' $taskProp) -Destination (Join-Path $taskOut '道具')
    }
    $taskReadme = @"
Moon·月 · Windows x64 试玩版
版本：$taskVersion（构建：$taskBuild；dirty表示包含本地尚未提交的修改）

1. 完整解压整个文件夹，再双击“Moon·月.exe”。不要在ZIP内直接运行，不要只发exe。
2. 请放在可写目录，例如桌面或下载目录；设置和诊断日志会保存在程序旁边。
3. 轻点头/脸/衣服可互动；长按或拖动可拎起；右键打开菜单、偏好设置或退出。
4. 空闲时会随机看书、翻页、散步。白天默认15分钟浅睡、20分钟熟睡；22:00～次日06:00为5/10分钟，均可在偏好中修改。
5. 本包可独立作为桌宠运行，不需要安装Kimi CLI、Node或Rust。未连接CLI时没有工作状态反馈。
6. “星光额度”需要本机已有Kimi登录凭证；没有登录时查询失败不影响桌宠互动。
7. 需要Windows 10 1903及以上的x64系统或Windows 11，以及Microsoft Edge WebView2 Runtime。若缺少WebView2，请安装微软官方运行时后重试。所需Visual C++运行库已随包提供，请保留exe旁的DLL文件。
8. 这是未签名的内部试玩版。若系统阻止启动，请先联系发送者核对来源；不必关闭安全软件。

9. “允许拉动与抱跑真实光标”默认关闭，需要在偏好中勾选并保存；互动时自行移动鼠标或按键即可结束。
10. 更新时先退出旧版，再把新包解压到新目录。要沿用偏好，仅复制旧版config/设置.json到新版config中；其余配置和素材使用新版文件。

11. 右键“小小琴盘”打开三排七阶琴键；首次点“启用琴盘声音”。支持自由弹奏、光流、地鼠与“听她弹”示范，音源可离线使用。
12. 更新前退出程序并备份个人数据。养成请一起复制养成存档.dat、养成存档.dat.bak、养成存档.migrated；琴盘记录为琴盘记录.json。只有养成存档.json的旧版，可将其复制到未运行过养成的新目录，首次启动会自动导入并保留旧档。勿复制.lock或.tmp文件；不要用旧版程序打开新版进度。

包内没有API密钥、登录凭证、个人设置、开发日志或美术源图。
13. 双击桌宠打开输入条，点击前端圆球收起/恢复，拖动圆球移动，Esc隐藏；Enter发送、Shift+Enter换行，不保存草稿。右键/托盘“交流设置”可添加常用服务商或自定义OpenAI兼容API连接，手填或查询模型、点击模型选用保存；多组Key分别加密，仅显示首尾四位。内部“原始记录”只读查看历史。隐藏输入不停止回复。文字聊天及生活互动API回应按服务商规则计费；未配置时离线功能照常使用。
14. 更新前退出程序，聊天历史可复制聊天记录.sqlite3到新目录；不要复制聊天记录.lock或临时journal。更换电脑或Windows账户需重新填写Key，勿把聊天凭据文件发送给他人。
15. 记忆默认关闭，在交流设置独立开启后采集新交流与真实生活事件，不自动整理旧历史。后台每批最多5轮问答和5个事件发给所选聊天服务商提炼，产生额外API用量；“她的记忆”浏览提炼结果、来源、修正和遗忘。向量检索在本机，首次需要下载约95MB公共中文模型，失败可在设置重试。聊天数据库含全部记忆，请保密并备份，勿用旧版程序打开新版数据库。模型缓存在程序旁“模型缓存”，本包不含个人数据或模型缓存；升级时可保留该目录免重复下载。
16. 自动备选默认关闭。为连接勾选“可作备选”、调整顺序并开启自动切换后，聊天、生活回应和记忆整理遇到网络/服务商错误可尝试其他已启用连接；同一份上下文可能发给多家并分别计费。聊天已输出部分文字、主动停止、清空后不自动重发。模型列表查询始终只访问指定连接。移除连接会删除对应加密Key，保留历史与记忆。
17. 角色正式名暗月，也接受小月。文字回复接收完并检查后显示，生成时保持等待；命中角色回退规则会使用本地台词，不额外调用模型。记忆中的可疑指令归入“待确认”，暂不用于回忆，可修正或删除。config/交流人设.json可调整人设、示例和规则，修改后重启。聊天数据库版本为$taskSchema，勿用旧版程序打开。规则会漏报或误报，不保证角色绝不被带偏。
18. 本版记忆支持冲突候选与旧版本留存；删除时可选择仅删提炼项、停止使用来源或同时删除关联原话。已有备份与服务商记录不会同步删除。旧数据库升级前会自动备份；升级失败不会重建空库。更新中不要混用旧程序或旧动画配置。
欢迎反馈：人物/气泡是否遮挡、拖拽是否自然、翻页是否连贯、缩放和多屏是否异常。
"@
    [IO.File]::WriteAllText((Join-Path $taskOut '试玩说明.txt'), $taskReadme, [Text.UTF8Encoding]::new($true))
    $taskZip = Join-Path $taskDist ($taskName + '.zip')
    if (Test-Path -LiteralPath $taskZip) { throw "压缩包已存在：$taskZip" }
    Compress-Archive -LiteralPath $taskOut -DestinationPath $taskZip -CompressionLevel Optimal
    $taskHash = (Get-FileHash -LiteralPath $taskZip -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText(($taskZip + '.sha256.txt'), ($taskHash + '  ' + [IO.Path]::GetFileName($taskZip) + "`r`n"), [Text.UTF8Encoding]::new($false))
    Get-Item -LiteralPath $taskZip | Select-Object FullName,Length
    Get-FileHash -LiteralPath $taskZip -Algorithm SHA256
} finally { Pop-Location }
