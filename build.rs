fn main() {
    // 只在 Windows 目标上嵌入资源
    #[cfg(target_os = "windows")]
    {
        embed_resource::compile("res/app.rc", embed_resource::NONE);
    }

    // 非 Windows 平台不做任何处理
    #[cfg(not(target_os = "windows"))]
    {
        let _ = ();
    }
}
