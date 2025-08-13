import * as THREE from 'three'
import { GLTFLoader } from 'three/examples/jsm/Addons.js'

interface AnimationData {
  name: string
  duration: number
  tracks: {
    name: string
    type: string
    times: number[]
    values: number[]
  }[]
}

interface AnimationConfig {
  extractDate: string
  totalAnimations: number
  animations: AnimationData[]
}

export class AnimationManager {
  private static instance: AnimationManager
  private animationConfig: AnimationConfig | null = null
  private originalAnimationScene: THREE.Group | null = null
  private loader = new GLTFLoader()

  static getInstance(): AnimationManager {
    if (!AnimationManager.instance) {
      AnimationManager.instance = new AnimationManager()
    }
    return AnimationManager.instance
  }

  // 원본 애니메이션 파일 로드
  async loadOriginalAnimations(path: string): Promise<void> {
    try {
      const gltf = await this.loader.loadAsync(path)
      this.originalAnimationScene = gltf.scene
      console.log(`Loaded ${gltf.animations?.length || 0} original animations`)
    } catch (error) {
      console.error('Failed to load original animations:', error)
    }
  }

  // JSON 애니메이션 데이터 로드
  async loadAnimationData(path: string): Promise<void> {
    try {
      const response = await fetch(path)
      this.animationConfig = await response.json()
      console.log(`Loaded ${this.animationConfig?.totalAnimations || 0} animation configs`)
    } catch (error) {
      console.error('Failed to load animation data:', error)
    }
  }

  // 정적 모델에 애니메이션 적용 (방법 1: 원본 파일 사용)
  applyOriginalAnimation(
    staticModel: THREE.Group, 
    animationIndex: number = 0
  ): THREE.AnimationMixer | null {
    if (!this.originalAnimationScene) {
      console.warn('Original animation scene not loaded')
      return null
    }

    // 원본 씬에서 애니메이션 찾기
    const originalGltf = this.originalAnimationScene.parent as { animations?: THREE.AnimationClip[] }
    if (!originalGltf?.animations || originalGltf.animations.length === 0) {
      console.warn('No animations found in original scene')
      return null
    }

    const animation = originalGltf.animations[animationIndex]
    if (!animation) {
      console.warn(`Animation ${animationIndex} not found`)
      return null
    }

    // 애니메이션 믹서 생성
    const mixer = new THREE.AnimationMixer(staticModel)
    
    try {
      const action = mixer.clipAction(animation)
      action.play()
      console.log(`Applied animation: ${animation.name}`)
      return mixer
    } catch (error) {
      console.error('Failed to apply animation:', error)
      return null
    }
  }

  // JSON 데이터로 애니메이션 생성 (방법 2: JSON 사용)
  createAnimationFromData(
    staticModel: THREE.Group,
    animationIndex: number = 0
  ): THREE.AnimationMixer | null {
    if (!this.animationConfig) {
      console.warn('Animation config not loaded')
      return null
    }

    const animData = this.animationConfig.animations[animationIndex]
    if (!animData) {
      console.warn(`Animation data ${animationIndex} not found`)
      return null
    }

    // JSON 데이터에서 Three.js 애니메이션 트랙 생성
    const tracks: THREE.KeyframeTrack[] = []
    
    animData.tracks.forEach(trackData => {
      let track: THREE.KeyframeTrack | null = null
      
      // 트랙 타입에 따라 적절한 KeyframeTrack 생성
      switch (trackData.type) {
        case 'VectorKeyframeTrack':
          track = new THREE.VectorKeyframeTrack(
            trackData.name,
            trackData.times,
            trackData.values
          )
          break
        case 'QuaternionKeyframeTrack':
          track = new THREE.QuaternionKeyframeTrack(
            trackData.name,
            trackData.times,
            trackData.values
          )
          break
        case 'NumberKeyframeTrack':
          track = new THREE.NumberKeyframeTrack(
            trackData.name,
            trackData.times,
            trackData.values
          )
          break
      }
      
      if (track) {
        tracks.push(track)
      }
    })

    if (tracks.length === 0) {
      console.warn('No valid tracks created')
      return null
    }

    // 애니메이션 클립 생성
    const animationClip = new THREE.AnimationClip(
      animData.name,
      animData.duration,
      tracks
    )

    // 믹서에 적용
    const mixer = new THREE.AnimationMixer(staticModel)
    const action = mixer.clipAction(animationClip)
    action.play()
    
    console.log(`Created animation from data: ${animData.name}`)
    return mixer
  }

  // 간단한 프로그래밍 애니메이션 (방법 3: 코드로 구현)
  createSimpleAnimation(staticModel: THREE.Group): {
    update: (time: number) => void
    start: () => void
    stop: () => void
  } {
    let isPlaying = false
    let startTime = 0

    return {
      start() {
        isPlaying = true
        startTime = Date.now()
      },
      
      stop() {
        isPlaying = false
      },
      
      update(_time: number) {
        if (!isPlaying) return
        
        const elapsed = (Date.now() - startTime) / 1000
        
        // 간단한 idle 애니메이션 (위아래로 부드럽게 움직임)
        staticModel.position.y = Math.sin(elapsed * 2) * 0.1
        
        // 회전 애니메이션
        staticModel.rotation.y = elapsed * 0.5
      }
    }
  }
}

// 사용 예시를 위한 유틸리티 함수들
export async function setupCharacterAnimation(
  staticModelPath: string,
  animationType: 'original' | 'json' | 'simple' = 'original'
): Promise<{
  model: THREE.Group
  mixer?: THREE.AnimationMixer
  simpleAnim?: { update: (time: number) => void; start: () => void; stop: () => void }
}> {
  const loader = new GLTFLoader()
  const animManager = AnimationManager.getInstance()
  
  // 정적 모델 로드
  const gltf = await loader.loadAsync(staticModelPath)
  const model = gltf.scene
  
  let mixer: THREE.AnimationMixer | undefined
  let simpleAnim: { update: (time: number) => void; start: () => void; stop: () => void } | undefined
  
  switch (animationType) {
    case 'original':
      // 원본 애니메이션 파일도 로드 (필요시)
      await animManager.loadOriginalAnimations('/models/girls_-_14_anims.glb')
      mixer = animManager.applyOriginalAnimation(model) ?? undefined
      break
      
    case 'json':
      // JSON 애니메이션 데이터 로드
      await animManager.loadAnimationData('/character_animations.json')
      mixer = animManager.createAnimationFromData(model) ?? undefined
      break
      
    case 'simple':
      // 간단한 프로그래밍 애니메이션
      simpleAnim = animManager.createSimpleAnimation(model)
      simpleAnim.start()
      break
  }
  
  return { model, mixer, simpleAnim }
}