// makeSplatStandardMaterial.ts
import * as THREE from 'three'

export type SplatLayer = {
  map: THREE.Texture // Albedo (sRGB)
  normalMap?: THREE.Texture // Normal (Linear)
  orm?: THREE.Texture // ORM: R=AO, G=Roughness, B=Metallic (Linear)
  tile: number
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
    shader.defines = { ...(shader.defines ?? {}), USE_UV: 1 }

    // Common uniforms
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

    // Optional uniforms
    const hasN = layers.some((l) => !!l.normalMap)
    const hasORM = layers.some((l) => !!l.orm)

    if (hasN) {
      if (layers[0].normalMap)
        shader.uniforms.normal0 = { value: layers[0].normalMap }
      if (layers[1].normalMap)
        shader.uniforms.normal1 = { value: layers[1].normalMap }
      if (layers[2].normalMap)
        shader.uniforms.normal2 = { value: layers[2].normalMap }
      if (layers[3].normalMap)
        shader.uniforms.normal3 = { value: layers[3].normalMap }
      shader.uniforms.normalScale = { value: 1.0 }
    }
    if (hasORM) {
      if (layers[0].orm) shader.uniforms.orm0 = { value: layers[0].orm }
      if (layers[1].orm) shader.uniforms.orm1 = { value: layers[1].orm }
      if (layers[2].orm) shader.uniforms.orm2 = { value: layers[2].orm }
      if (layers[3].orm) shader.uniforms.orm3 = { value: layers[3].orm }
    }

    // Vertex shader: splat UV
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

    // Fragment shader: declarations
    shader.fragmentShader = shader.fragmentShader
      .replace(
        '#include <map_pars_fragment>',
        `#include <map_pars_fragment>
         uniform sampler2D splatMap;
         uniform sampler2D diffuse0, diffuse1, diffuse2, diffuse3;
         uniform float tile0, tile1, tile2, tile3;
         varying vec2 vUvSplat;
         ${hasN ? 'uniform sampler2D normal0, normal1, normal2, normal3; uniform float normalScale;' : ''}
         ${hasORM ? 'uniform sampler2D orm0, orm1, orm2, orm3;' : ''}`
      )
      // Albedo blending
      .replace(
        '#include <map_fragment>',
        `vec4 weights = texture2D(splatMap, vUvSplat);
         float wSum = weights.r + weights.g + weights.b + weights.a;
         if (wSum > 1e-5) weights /= wSum;

         vec3 c0 = texture2D(diffuse0, vUv * tile0).rgb;
         vec3 c1 = texture2D(diffuse1, vUv * tile1).rgb;
         vec3 c2 = texture2D(diffuse2, vUv * tile2).rgb;
         vec3 c3 = texture2D(diffuse3, vUv * tile3).rgb;
         vec3 blended = c0*weights.r + c1*weights.g + c2*weights.b + c3*weights.a;
         diffuseColor = vec4(blended, 1.0);`
      )
      // Inject custom normal perturbation function
      .replace(
        '#include <normal_pars_fragment>',
        `#include <normal_pars_fragment>
      ${
        hasN
          ? `
        vec3 perturbNormal2Arb_custom( vec3 eye_pos, vec3 surf_norm, vec3 mapN, float faceDir ) {
        vec3 q0 = dFdx( eye_pos.xyz );
        vec3 q1 = dFdy( eye_pos.xyz );
        vec2 st0 = dFdx( vUv );
        vec2 st1 = dFdy( vUv );
        vec3 S = normalize( q0 * st1.t - q1 * st0.t );
        vec3 T = normalize( -q0 * st1.s + q1 * st0.s );
        vec3 N = normalize( surf_norm );
        N *= faceDir;
        mapN.xy *= ( faceDir * normalScale );
        mat3 tsn = mat3( S, T, N );
        return normalize( tsn * mapN );
      }`
          : ``
      }`
      )
      // Normal blending
      .replace(
        '#include <normal_fragment_maps>',
        hasN
          ? `
        vec4 wNrm = texture2D(splatMap, vUvSplat);
        float sN = wNrm.r + wNrm.g + wNrm.b + wNrm.a; if (sN > 1e-5) wNrm /= sN;
  
        vec3 n0 = texture2D(normal0, vUv * tile0).xyz * 2.0 - 1.0;
        vec3 n1 = texture2D(normal1, vUv * tile1).xyz * 2.0 - 1.0;
        vec3 n2 = texture2D(normal2, vUv * tile2).xyz * 2.0 - 1.0;
        vec3 n3 = texture2D(normal3, vUv * tile3).xyz * 2.0 - 1.0;
        vec3 mapN = normalize(n0*wNrm.r + n1*wNrm.g + n2*wNrm.b + n3*wNrm.a);
  
        normal = perturbNormal2Arb_custom(-vViewPosition, normal, mapN, (gl_FrontFacing) ? 1.0 : -1.0);`
          : `#include <normal_fragment_maps>`
      )
      // Roughness (G channel)
      .replace(
        '#include <roughnessmap_fragment>',
        hasORM
          ? `
        float roughnessFactor = roughness;
        vec4 wR = texture2D(splatMap, vUvSplat);
        float sR = wR.r + wR.g + wR.b + wR.a; if (sR > 1e-5) wR /= sR;
  
        float r0 = texture2D(orm0, vUv * tile0).g;
        float r1 = texture2D(orm1, vUv * tile1).g;
        float r2 = texture2D(orm2, vUv * tile2).g;
        float r3 = texture2D(orm3, vUv * tile3).g;
        float rBlend = r0*wR.r + r1*wR.g + r2*wR.b + r3*wR.a;
  
        roughnessFactor = roughnessFactor * rBlend;`
          : `#include <roughnessmap_fragment>`
      )
      // Metalness (B channel)
      .replace(
        '#include <metalnessmap_fragment>',
        hasORM
          ? `
        float metalnessFactor = metalness;
        vec4 wM = texture2D(splatMap, vUvSplat);
        float sM = wM.r + wM.g + wM.b + wM.a; if (sM > 1e-5) wM /= sM;
  
        float m0 = texture2D(orm0, vUv * tile0).b;
        float m1 = texture2D(orm1, vUv * tile1).b;
        float m2 = texture2D(orm2, vUv * tile2).b;
        float m3 = texture2D(orm3, vUv * tile3).b;
        float mBlend = m0*wM.r + m1*wM.g + m2*wM.b + m3*wM.a;
  
        metalnessFactor = metalnessFactor * mBlend;`
          : `#include <metalnessmap_fragment>`
      )
      // AO (R channel)
      .replace(
        '#include <aomap_fragment>',
        hasORM
          ? `
        vec4 wAO = texture2D(splatMap, vUvSplat);
        float sAO = wAO.r + wAO.g + wAO.b + wAO.a; if (sAO > 1e-5) wAO /= sAO;
  
        float ao0v = texture2D(orm0, vUv * tile0).r;
        float ao1v = texture2D(orm1, vUv * tile1).r;
        float ao2v = texture2D(orm2, vUv * tile2).r;
        float ao3v = texture2D(orm3, vUv * tile3).r;
        float aoBlend = ao0v*wAO.r + ao1v*wAO.g + ao2v*wAO.b + ao3v*wAO.a;
  
        reflectedLight.indirectDiffuse *= aoBlend;`
          : `#include <aomap_fragment>`
      )
  }

  // To change tiles/textures without recreating the material,
  // store values like mat.userData.tiles = [ ... ] and update uniforms as needed.

  return mat
}
