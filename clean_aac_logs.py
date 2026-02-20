import re
import sys

def clean_file(path):
    with open(path, 'r', encoding='utf-8') as f:
        content = f.read()

    # Remove trace log definition and usage
    content = re.sub(r'static AAC_TRACE_ENABLED: OnceLock<bool> = OnceLock::new\(\);\nstatic AAC_TRACE_COUNT: AtomicUsize = AtomicUsize::new\(0\);\nconst AAC_TRACE_LIMIT: usize = \d+;\n\nfn aac_trace_enabled\(\) -> bool \{.*?\}\n\nfn aac_trace_log\(message: impl FnOnce\(\) -> String\) \{.*?\}\n', '', content, flags=re.DOTALL)
    
    # Remove aac_trace_log(|| { ... }); blocks
    content = re.sub(r'[ \t]*aac_trace_log\(\|\| \{.*?\}\);\n', '', content, flags=re.DOTALL)
    
    # Remove debug!(...)
    content = re.sub(r'[ \t]*debug!\(\n.*?\);\n', '', content, flags=re.DOTALL)
    content = re.sub(r'[ \t]*debug!\(.*?\);\n', '', content, flags=re.DOTALL)

    # Remove import of debug
    content = re.sub(r'use log::\{debug, info\};\n', 'use log::info;\n', content)

    with open(path, 'w', encoding='utf-8') as f:
        f.write(content)

clean_file('crates/tao-codec/src/decoders/aac/mod.rs')
