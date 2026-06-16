@echo off
chcp 65001 >nul
setlocal enabledelayedexpansion

echo ======================================================
echo   Qwen Launcher Safe -- Windows 手动验证脚本
echo ======================================================
echo.

set EXE=%~dp0qwen-launcher-safe.exe
if not exist "%EXE%" (
    echo [错误] 未找到 %EXE%
    exit /b 1
)

echo [1/6] 测试 --help（查看所有子命令）
echo --------------------------------------------------
"%EXE%" --help
echo.

echo [2/6] 测试 init 别名是否存在（应显示 init-config 信息）
echo --------------------------------------------------
"%EXE%" init --help
echo.

echo [3/6] 测试 init-config 子命令帮助
echo --------------------------------------------------
"%EXE%" init-config --help
echo.

echo [4/6] 测试 init-config --show（无配置时显示未设置）
echo --------------------------------------------------
"%EXE%" init-config --show
echo.

echo [5/6] 测试 init-config --qwen-path auto（配置自动搜索）
echo --------------------------------------------------
"%EXE%" init-config --qwen-path auto
echo.

echo [6/6] 再次测试 init-config --show（应显示已配置）
echo --------------------------------------------------
"%EXE%" init-config --show
echo.

echo ======================================================
echo  验证完成！如果所有命令都正常输出则为 [OK] 通过
echo ======================================================
echo.
pause
