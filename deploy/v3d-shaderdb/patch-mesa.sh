#!/bin/sh
# Make V3D_DEBUG=shaderdb output attributable to wgpu pipelines:
#  - SHADER-DB names carry the SPIR-V entry point after the pipeline hash
#    (v3dv otherwise replaces the name with the hash alone);
#  - with MESA_LOG_PIPELINE_NAMES set, every vkSetDebugUtilsObjectNameEXT on a
#    pipeline is logged, so wgpu's labels (set right after creation, with
#    WGPU_DEBUG=1) follow the SHADER-DB lines of their pipeline.
set -eu
cd /mesa

f=src/broadcom/vulkan/v3dv_pipeline.c
grep -q 'nir->info.name = ralloc_strdup(nir, blake3buf);' $f
sed -i 's|nir->info.name = ralloc_strdup(nir, blake3buf);|nir->info.name = ralloc_asprintf(nir, "%s:%s", blake3buf, stage->entrypoint ? stage->entrypoint : "?");|' $f

f=src/vulkan/runtime/vk_debug_utils.c
python3 - "$f" <<'EOF'
import sys
path = sys.argv[1]
src = open(path).read()
anchor = """vk_common_SetDebugUtilsObjectNameEXT(
   VkDevice _device,
   const VkDebugUtilsObjectNameInfoEXT *pNameInfo)
{
   VK_FROM_HANDLE(vk_device, device, _device);
"""
assert src.count(anchor) == 1, "anchor not found"
src = src.replace(anchor, anchor + """
   if (pNameInfo->objectType == VK_OBJECT_TYPE_PIPELINE &&
       getenv("MESA_LOG_PIPELINE_NAMES"))
      mesa_logi("PIPELINE-NAME 0x%llx %s",
                (unsigned long long)pNameInfo->objectHandle,
                pNameInfo->pObjectName ? pNameInfo->pObjectName : "(null)");
""")
src = '#include <stdlib.h>\n#include "util/log.h"\n' + src
open(path, "w").write(src)
EOF
