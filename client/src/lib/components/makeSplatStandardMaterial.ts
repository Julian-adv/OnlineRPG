// makeSplatStandardMaterial.ts
import * as THREE from 'three'

export type SplatLayer = {
  map: THREE.Texture // Albedo (color) tile texture
  tile: number // Tiling repeat (e.g., 8 = 8x repeat)
}

export type SplatParams = {
  layers: [SplatLayer, SplatLayer, SplatLayer, SplatLayer] // RGBA order
  splatMap: THREE.Texture // RGBA weight map (R=layer0, G=layer1, B=layer2, A=layer3)
  splatScale?: number // UV scale of the splat map (default 1)
}

export function makeSplatStandardMaterial({
  layers,
  splatMap,
  splatScale = 1,
}: SplatParams) {
  // Standard material: keep lighting/shadows/physical properties intact
  const mat = new THREE.MeshStandardMaterial({
    color: 0xffffff,
    roughness: 1.0, // Sensible default; adjust externally as needed
    metalness: 0.0,
  })

  // Recommended common texture settings
  const prepare = (t: THREE.Texture, isColor = false) => {
    t.wrapS = t.wrapT = THREE.RepeatWrapping
    t.anisotropy = 8
    if (isColor) t.colorSpace = THREE.SRGBColorSpace // Albedo uses sRGB
    t.needsUpdate = true
  }

  layers.forEach((l) => prepare(l.map, true))
  prepare(splatMap, false)
  // Splat map filtering: Linear for smooth blends, Nearest for hard edges
  splatMap.minFilter = THREE.LinearMipMapLinearFilter
  splatMap.magFilter = THREE.LinearFilter

  mat.onBeforeCompile = (shader) => {
    // Ensure UV varyings are generated even if no base map is set
    shader.defines = { ...(shader.defines ?? {}), USE_UV: 1 }

    // Inject uniforms
    shader.uniforms.splatMap = { value: splatMap }
    shader.uniforms.diffuse0 = { value: layers[0].map }
    shader.uniforms.diffuse1 = { value: layers[1].map }
    shader.uniforms.diffuse2 = { value: layers[2].map }
    shader.uniforms.diffuse3 = { value: layers[3].map }
    shader.uniforms.tile0 = { value: layers[0].tile }
    shader.uniforms.tile1 = { value: layers[1].tile }
    shader.uniforms.tile2 = { value: layers[2].tile }
    shader.uniforms.tile3 = { value: layers[3].tile }
    shader.uniforms.splatScale = { value: splatScale }

    // Vertex: pass dedicated UVs for splat
    shader.vertexShader = shader.vertexShader
      .replace(
        '#include <uv_pars_vertex>',
        `#include <uv_pars_vertex>
         uniform float splatScale;
         varying vec2 vUvSplat;`
      )
      .replace(
        '#include <uv_vertex>',
        `#include <uv_vertex>
         vUvSplat = uv * splatScale;`
      )

    // Fragment: color blending (customize albedo only)
    shader.fragmentShader = shader.fragmentShader
      .replace(
        '#include <map_pars_fragment>',
        `#include <map_pars_fragment>
         uniform sampler2D splatMap;
         uniform sampler2D diffuse0;
         uniform sampler2D diffuse1;
         uniform sampler2D diffuse2;
         uniform sampler2D diffuse3;
         uniform float tile0;
         uniform float tile1;
         uniform float tile2;
         uniform float tile3;
         varying vec2 vUvSplat;`
      )
      .replace(
        // Intercept default map sampling and replace with our blending result
        '#include <map_fragment>',
        `
         vec4 weights = texture2D(splatMap, vUvSplat);
         // Normalize so weights sum to 1 (avoid darkening/brightening)
         float wSum = weights.r + weights.g + weights.b + weights.a;
         if (wSum > 0.0001) weights /= wSum;

         vec3 c0 = texture2D(diffuse0, vUv * tile0).rgb;
         vec3 c1 = texture2D(diffuse1, vUv * tile1).rgb;
         vec3 c2 = texture2D(diffuse2, vUv * tile2).rgb;
         vec3 c3 = texture2D(diffuse3, vUv * tile3).rgb;

         vec3 blended = c0 * weights.r + c1 * weights.g + c2 * weights.b + c3 * weights.a;

         // Inject into the standard material's diffuseColor
         diffuseColor = vec4(blended, 1.0);
        `
      )
  }

  // To change tiles/textures without recreating the material,
  // store values like mat.userData.tiles = [ ... ] and update uniforms as needed.

  return mat
}
