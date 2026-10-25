//! 有界旁路执行协议，取消句柄由请求方持有

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ExecOptions {
    pub timeout: Duration,
    /// 本地诊断和测试标识，不发送到服务器
    pub label: Option<&'static str>,
    pub max_output_bytes: usize,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            label: None,
            timeout: Duration::from_secs(10),
            max_output_bytes: 256 * 1024,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecFailure {
    Disconnected,
    Unsupported,
    Timeout,
    Cancelled,
    OutputLimit,
    Busy,
    Transport(String),
}

pub type DetailedExecResult = Result<ExecOutput, ExecFailure>;

/// 通道关闭不等于驱动中的进程已退出，调用方必须保留超时暂停状态
#[non_exhaustive]
pub struct DetailedExecRequest {
    pub command: String,
    pub options: ExecOptions,
    pub reply: mpsc::Sender<DetailedExecResult>,
    cancelled: Arc<AtomicBool>,
    input: Option<crate::connection::ConnectionSecret>,
}

impl DetailedExecRequest {
    pub fn new(command: String, options: ExecOptions) -> (Self, ExecHandle) {
        let (reply, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        (
            Self {
                command,
                options,
                reply,
                cancelled: cancelled.clone(),
                input: None,
            },
            ExecHandle {
                receiver,
                cancelled,
            },
        )
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// 敏感输入只经 SSH 标准输入发送，不进入命令文本
    pub fn with_input(mut self, input: crate::connection::ConnectionSecret) -> Self {
        self.input = Some(input);
        self
    }

    pub(crate) fn input(&self) -> Option<&str> {
        self.input.as_ref().map(|input| input.expose())
    }

    pub(crate) fn redact(&self, output: String) -> String {
        match self
            .input()
            .map(|input| input.trim_end_matches(['\r', '\n']))
        {
            Some(input) if !input.is_empty() => output.replace(input, "<redacted>"),
            _ => output,
        }
    }
}

pub struct ExecHandle {
    receiver: mpsc::Receiver<DetailedExecResult>,
    cancelled: Arc<AtomicBool>,
}

impl ExecHandle {
    pub fn try_recv(&self) -> Option<DetailedExecResult> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(ExecFailure::Disconnected)),
        }
    }
}

impl Drop for ExecHandle {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// 将脚本交给 POSIX sh，避免登录 shell 对引号和历史展开的差异
pub fn shell_script(script: &str) -> String {
    let encoded = encode_script(script);
    format!("sh -c 'printf \"%b\" \"{encoded}\" | sh'")
}

/// 保留标准输入供脚本内的 sudo 等命令读取
pub fn shell_script_with_input(script: &str) -> String {
    let encoded = encode_script(script);
    format!("sh -c 'exec sh -c \"$(printf \"%b\" \"{encoded}\")\"'")
}

fn encode_script(script: &str) -> String {
    script
        .bytes()
        .map(|byte| format!("\\0{byte:03o}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn encoded_script_preserves_stdin_and_literal_shell_characters() {
        use std::{
            io::Write as _,
            process::{Command, Stdio},
        };
        let script =
            "IFS= read -r password; printf '%s\\n' \"$password\"; printf '%s' \"literal ' ! \\\"\"";
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(shell_script_with_input(script))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"secret $() ' \\\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "secret $() ' \\\nliteral ' ! \""
        );
    }

    #[test]
    fn dropping_the_handle_cancels_a_queued_request() {
        let (request, handle) = DetailedExecRequest::new("true".into(), ExecOptions::default());
        assert!(!request.is_cancelled());
        drop(handle);
        assert!(request.is_cancelled());
    }
}
