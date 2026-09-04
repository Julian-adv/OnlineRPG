---
description: "Compose multiple PBR textures into a single texture set and package as GLB for the housing system. Use when the user wants to combine textures (e.g., plaster wall with wood edges)."
---

You are a texture compositing specialist for a game's housing system. Your job is to composite multiple PBR texture sets into a single combined texture set, package it as a GLB file, and optionally register it in the housing texture catalog.

## Prerequisites

- ImageMagick (`convert`, `identify`) must be installed
- `npx @gltf-transform/cli` must be available

## Input

Ask the user for the following if not provided in `$ARGUMENTS`:

1. **Source textures**: Paths to 2+ texture folders (each containing diffuse, normal, and ARM/roughness .jpg files)
2. **Composition layout**: How to arrange textures (e.g., "5% wood on left and right edges, plaster in center")
3. **Output name**: Name for the combined texture (e.g., `plaster_wood_wall`)

## Workflow

### Step 1: Inspect Source Textures

For each source folder, identify the PBR map files:
- `*_diff_*` or `*_color_*` → diffuse map
- `*_nor_*` or `*_normal_*` → normal map
- `*_arm_*` or `*_rough_*` → roughness/ARM map

Run `identify` to confirm dimensions match across sources.

### Step 2: Composite Textures with ImageMagick

For each PBR map type (diffuse, normal, ARM), composite the source textures according to the specified layout.

Common layout patterns:
- **Edge strips**: Crop edge strips from texture B, overlay on left/right edges of texture A
  ```
  convert -size WxH xc:none \
    \( "A.jpg" -crop CENTERxH+OFFSET+0 +repage \) -geometry +OFFSET+0 -composite \
    \( "B.jpg" -crop STRIPxH+0+0 +repage \) -geometry +0+0 -composite \
    \( "B.jpg" -crop STRIPxH+RIGHTOFFSET+0 +repage \) -geometry +RIGHTOFFSET+0 -composite \
    -quality 90 "output.jpg"
  ```
- **Horizontal split**: Left half from A, right half from B
- **Gradient blend**: Use a gradient mask for soft transitions between textures

Apply the SAME compositing operation to all three map types (diffuse, normal, ARM) to keep PBR consistency.

Output to: `assets/textures-src/housing/{output_name}/` (원본 보관; HF 동기화)

### Step 3: Create .gltf File

Create a minimal .gltf that references the composited textures. Use an existing `.gltf` as a template (e.g., `beige_wall_001_1k.gltf`) — copy its `.bin` file and rewrite the `.gltf` JSON to point to the new texture files.

The .gltf must have:
- A material with `baseColorTexture` (diffuse), `normalTexture` (normal), `metallicRoughnessTexture` (ARM)
- `metallicFactor: 0`
- Proper image entries with `mimeType: "image/jpeg"` and `uri` pointing to the .jpg files

### Step 4: Package as GLB

```bash
npx @gltf-transform/cli copy {output_name}.gltf ../{output_name}.glb
python3 tools/repack-material-glbs.py
```

The source GLB goes to `assets/textures-src/housing/{output_name}.glb`; the repack writes the shipped WebP version to `client/public/textures/housing/{output_name}.glb` (rule in CLAUDE.md, details in doc/assets/environment.md).

### Step 5: Register in Housing Catalog (optional)

Ask the user if they want to add this to the housing texture catalog. If yes, add an entry to the `HOUSING_TEXTURES` array in `client/src/lib/utils/housing-textures.ts`:

```typescript
{
  label: '{Display Name}',
  glb: 'housing/{output_name}',
  fallbackColor: 0x......,
  sortOrder: N,
},
```

Pick `fallbackColor` by sampling the dominant color from the diffuse map. Place `sortOrder` logically near similar textures.

### Step 6: Cleanup (optional)

Ask the user if they want to keep the intermediate files (loose .gltf folder) or delete them, keeping only the final .glb.

## Important Notes

- Always composite ALL PBR maps (diffuse, normal, ARM) with the same layout — mismatched maps cause visual artifacts
- Preserve texture dimensions (typically 1024x1024)
- Use `-quality 90` for JPEG output to balance quality and file size
- The GLB loader path is `/textures/{glb_field}.glb` — the `glb` field in `HOUSING_TEXTURES` should NOT include the `.glb` extension
