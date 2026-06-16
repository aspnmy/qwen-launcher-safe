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

echo [2/6] 测试 init 别名
echo --------------------------------------------------
"%EXE%" init --help
echo.

echo [3/6] 测试 init-config --help 选项
echo --------------------------------------------------
"%EXE%" init-config --help
echo.

echo [4/6] 测试 init-config --show（显示当前配置）
echo --------------------------------------------------
"%EXE%" init-config --show
echo.

echo [5/6] 测试 init-config --qwen-path 设置路径
echo --------------------------------------------------
"%EXE%" init-config --qwen-path "C:\Users\nasAdmin\.cherrystudio\bin\qwen.exe"
echo.

echo [6/6] 再次测试 init-config --show（显示已配置）
echo --------------------------------------------------
"%EXE%" init-config --show
echo.

echo ======================================================
echo  验证完成！如果所有命令都正常输出则为 [OK] 通过
echo ======================================================
echo.
pause
