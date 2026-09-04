import React, { useEffect, useRef } from 'react'

interface GargantuaCanvasProps {
  isDark: boolean
  onFpsUpdate?: (fps: number) => void
}

const VS_SRC = `attribute vec2 position;
void main() { gl_Position = vec4(position, 0.0, 1.0); }`

const FS_SRC = `precision highp float;
uniform float uTime;
uniform vec2 uResolution;
uniform vec2 uMouse;
uniform float uTheme;
uniform vec2 uSingularityCenter;
uniform vec3 uTintA;
uniform vec3 uTintB;
uniform vec3 uTintC;

const float DISK_IN = 2.5;
const float DISK_OUT = 11.8;
const float CAM_DIST = 23.5;
const float FOCAL = 1.30;
const int STEPS = 64;

vec3 mod289(vec3 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec4 mod289(vec4 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec4 permute(vec4 x) { return mod289(((x * 34.0) + 1.0) * x); }
vec4 taylorInvSqrt(vec4 r) { return 1.79284291400159 - 0.85373472095314 * r; }

float snoise(vec3 v) {
  const vec2 C = vec2(1.0 / 6.0, 1.0 / 3.0);
  const vec4 D = vec4(0.0, 0.5, 1.0, 2.0);
  vec3 i = floor(v + dot(v, C.yyy));
  vec3 x0 = v - i + dot(i, C.xxx);
  vec3 g = step(x0.yzx, x0.xyz);
  vec3 l = 1.0 - g;
  vec3 i1 = min(g.xyz, l.zxy);
  vec3 i2 = max(g.xyz, l.zxy);
  vec3 x1 = x0 - i1 + C.xxx;
  vec3 x2 = x0 - i2 + C.yyy;
  vec3 x3 = x0 - D.yyy;
  i = mod289(i);
  vec4 p = permute(permute(permute(i.z + vec4(0.0, i1.z, i2.z, 1.0)) + i.y + vec4(0.0, i1.y, i2.y, 1.0)) + i.x + vec4(0.0, i1.x, i2.x, 1.0));
  float n_ = 0.142857142857;
  vec3 ns = n_ * D.wyz - D.xzx;
  vec4 j = p - 49.0 * floor(p * ns.z * ns.z);
  vec4 x_ = floor(j * ns.z);
  vec4 y_ = floor(j - 7.0 * x_);
  vec4 x = x_ * ns.x + ns.yyyy;
  vec4 y = y_ * ns.x + ns.yyyy;
  vec4 h = 1.0 - abs(x) - abs(y);
  vec4 b0 = vec4(x.xy, y.xy);
  vec4 b1 = vec4(x.zw, y.zw);
  vec4 s0 = floor(b0) * 2.0 + 1.0;
  vec4 s1 = floor(b1) * 2.0 + 1.0;
  vec4 sh = -step(h, vec4(0.0));
  vec4 a0 = b0.xzyw + s0.xzyw * sh.xxyy;
  vec4 a1 = b1.xzyw + s1.xzyw * sh.zzww;
  vec3 p0 = vec3(a0.xy, h.x);
  vec3 p1 = vec3(a0.zw, h.y);
  vec3 p2 = vec3(a1.xy, h.z);
  vec3 p3 = vec3(a1.zw, h.w);
  vec4 norm = taylorInvSqrt(vec4(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
  p0 *= norm.x; p1 *= norm.y; p2 *= norm.z; p3 *= norm.w;
  vec4 m = max(0.6 - vec4(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), 0.0);
  m = m * m;
  return 42.0 * dot(m * m, vec4(dot(p0, x0), dot(p1, x1), dot(p2, x2), dot(p3, x3)));
}

float hash21(vec2 p) {
  p = fract(p * vec2(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}

vec4 diskShade(vec3 hit, vec3 rd, float theme) {
  float hr = length(hit.xz);
  float ang = atan(hit.z, hit.x);
  
  float omega = 3.0 * pow(hr, -1.5);
  float sa = ang + uTime * omega;
  
  float radialFlow = mod(uTime * 1.6, 31.416) * pow(clamp(hr, 1.0, 10.0), -0.65);

  vec3 np1 = vec3(cos(sa) * 1.3, sin(sa) * 1.3, log(hr) * 5.2 + radialFlow);
  vec3 np2 = vec3(cos(sa * 2.2 - uTime * 0.12) * 2.4, sin(sa * 2.2) * 2.4, log(hr) * 8.6 + radialFlow * 1.3);
  float n1 = snoise(np1) * 0.5 + 0.5;
  float n2 = snoise(np2) * 0.5 + 0.5;
  float streak = pow(n1 * 0.62 + n2 * 0.38, 2.0) * 1.4;
  
  float wInflow = 1.0 - smoothstep(0.15, 0.85, theme);
  float wOutflow = smoothstep(0.15, 0.85, theme);

  float spiral = sin(ang * 3.0 - log(hr) * 8.5 + uTime * 3.2);
  float innerPull = smoothstep(DISK_IN + 3.0, DISK_IN, hr);
  float tidalInflow = smoothstep(0.45, 0.95, spiral) * innerPull * 1.5 * wInflow;

  float wavePhase = sin(hr * 4.4 - uTime * 5.0);
  float corePush = smoothstep(DISK_OUT, DISK_IN, hr);
  float ejectBurst = smoothstep(0.25, 0.92, wavePhase) * corePush * 1.4 * wOutflow;
  
  float hotspot = smoothstep(0.70, 0.98, n1 * n2);
  
  float fade = smoothstep(DISK_IN, DISK_IN + 0.8, hr) * (1.0 - smoothstep(7.8, DISK_OUT, hr));
  float density = fade * pow(DISK_IN / hr, 1.55);
  float t = clamp((hr - DISK_IN) / (DISK_OUT - DISK_IN), 0.0, 1.0);
  
  vec3 tangent = normalize(vec3(-hit.z, 0.0, hit.x));
  float dop = dot(tangent, normalize(rd));
  float beam = pow(clamp(1.0 - 0.20 * dop, 0.7, 1.4), 1.3);

  float flick = 0.94 + 0.06 * sin(uTime * 1.2 + sin(uTime * 0.4) * 2.5);
  float e = density * streak * beam * flick * (1.0 + 1.4 * hotspot);

  float photonRing = exp(-pow(abs(hr - (DISK_IN + 0.35)) * 5.2, 2.0)) * 1.7;

  // 亮色模式：炽烈相对论黑洞
  vec3 colInner = vec3(1.0, 0.98, 0.92);
  vec3 colFire  = vec3(1.0, 0.48, 0.08);
  vec3 colRuby  = vec3(0.96, 0.18, 0.08);
  vec3 colDeep  = vec3(0.88, 0.12, 0.16);

  vec3 diskColorLight = mix(colInner, colFire, smoothstep(0.0, 0.20, t));
  diskColorLight = mix(diskColorLight, colRuby, smoothstep(0.18, 0.55, t));
  diskColorLight = mix(diskColorLight, colDeep, smoothstep(0.50, 1.0, t));
  diskColorLight += vec3(0.20, 0.12, 0.0) * clamp(-dop * 0.5, 0.0, 0.5);
  diskColorLight = mix(diskColorLight, vec3(0.78, 0.12, 0.08), clamp(dop * 0.45, 0.0, 0.45));
  diskColorLight += colInner * photonRing * 1.1 + colFire * tidalInflow * 0.9;

  // 暗色模式：高能逆喷涌白洞
  float shock = sin(hr * 4.2 - uTime * 4.0) * 0.5 + 0.5;
  float pulse = sin(uTime * 2.2) * 0.18 + 0.82;
  
  vec3 whCore = vec3(1.4, 1.5, 1.8);
  vec3 whCyan = mix(uTintA, vec3(0.25, 0.95, 1.0), 0.65);
  vec3 whPink = mix(uTintB, uTintC, 0.55);
  vec3 whDeep = vec3(0.08, 0.02, 0.22);

  vec3 diskColorDark = mix(whCore, whCyan, smoothstep(0.0, 0.25, t));
  diskColorDark = mix(diskColorDark, whPink, smoothstep(0.22, 0.70, t));
  diskColorDark = mix(diskColorDark, whDeep, smoothstep(0.65, 1.0, t));
  diskColorDark += (whCore * shock * 0.35 + whCyan * 0.4) * pulse;
  diskColorDark += whCore * photonRing * 1.3 + whCore * ejectBurst * 0.85;

  vec3 finalRgb = mix(diskColorLight * e * 1.65, diskColorDark * e * 1.55, theme);
  float alpha = clamp(e * (mix(2.3, 1.7, 1.0 - theme)) + photonRing * 0.7, 0.0, 0.99);
  return vec4(finalRgb, alpha);
}

vec3 background(vec3 dir, float theme) {
  vec3 d = normalize(dir);
  vec2 sc = vec2(atan(d.z, d.x) * 3.2, asin(clamp(d.y, -1.0, 1.0)) * 6.4);
  
  vec2 cell = floor(sc * 14.0);
  vec2 cuv = fract(sc * 14.0);
  float sh = hash21(cell);
  vec2 spos = vec2(hash21(cell + 7.13), hash21(cell + 3.71));
  float sd = length(cuv - spos);
  float star1 = step(0.90, sh) * exp(-sd * sd * 420.0);
  float tw1 = 0.58 + 0.42 * sin(uTime * 0.85 + sh * 42.0);

  vec2 sc2 = sc + vec2(17.3, 9.1);
  vec2 cell2 = floor(sc2 * 26.0);
  float sh2 = hash21(cell2);
  vec2 spos2 = vec2(hash21(cell2 + 3.1), hash21(cell2 + 8.4));
  float sd2 = length(fract(sc2 * 26.0) - spos2);
  float star2 = step(0.85, sh2) * exp(-sd2 * sd2 * 540.0);
  float tw2 = 0.50 + 0.50 * sin(uTime * 1.15 + sh2 * 35.0);

  float totalStars = star1 * tw1 + star2 * tw2 * 0.60;

  vec3 starDark = mix(uTintA, uTintB, 0.35 + 0.45 * sh) * totalStars * 1.15;
  float nn = snoise(d * 2.6) * 0.5 + 0.5;
  float nn2 = snoise(d * 5.2 + vec3(uTime * 0.02)) * 0.5 + 0.5;
  vec3 colDark = vec3(0.012, 0.015, 0.024) + starDark + mix(uTintA, uTintC, 0.35) * 0.04 * nn * nn;

  vec3 colLight = vec3(0.968, 0.972, 0.982);
  vec3 nebulaCloud = mix(vec3(0.052, 0.038, 0.025), vec3(0.032, 0.038, 0.060), d.x * 0.5 + 0.5);
  colLight -= nebulaCloud * (nn * 0.72 + nn2 * 0.28) * 0.09;

  float vignette = smoothstep(0.4, 1.4, length(d.xy));
  colLight -= vec3(0.014, 0.013, 0.011) * vignette;

  vec3 starGemLight = mix(vec3(0.92, 0.20, 0.06), vec3(0.66, 0.05, 0.14), sh);
  colLight = mix(colLight, starGemLight, clamp(totalStars * 0.88, 0.0, 1.0));

  return mix(colLight, colDark, theme);
}

vec4 calcMeteors(vec2 uv, float time, float theme) {
  vec4 total = vec4(0.0);
  
  // 轨道 1
  {
    float period = 9.6;
    float localT = mod(time + 17.3, period);
    if (localT < 2.0) {
      float prog = localT / 2.0;
      float ang = -0.56;
      float ca = cos(ang), sa = sin(ang);
      vec2 rotUv = vec2(uv.x * ca - uv.y * sa, uv.x * sa + uv.y * ca);
      
      float headX = mix(-1.7, 1.8, prog);
      float trackY = 0.22;
      float dx = rotUv.x - headX;
      float dy = abs(rotUv.y - trackY);
      float trailLen = 0.78;
      
      if (dx < 0.04 && dx > -trailLen && dy < 0.005) {
        float headCore = exp(-dx * dx * 2000.0) * exp(-dy * dy * 55000.0);
        float tailFade = pow(max(0.0, 1.0 + dx / trailLen), 2.0);
        float trailGlow = tailFade * exp(-dy * dy * 32000.0);
        float mIntensity = headCore * 2.2 + trailGlow * 1.35;
        
        vec3 colLight = mix(vec3(0.96, 0.22, 0.06), vec3(1.0, 0.98, 0.88), clamp(headCore * 1.4, 0.0, 1.0));
        vec3 colDark = mix(vec3(0.25, 0.92, 1.0), vec3(1.0, 1.0, 1.0), clamp(headCore * 1.4, 0.0, 1.0));
        vec3 mCol = mix(colLight, colDark, theme);
        total += vec4(mCol * mIntensity, clamp(mIntensity * 0.95, 0.0, 1.0));
      }
    }
  }
  
  // 轨道 2
  {
    float period = 13.2;
    float localT = mod(time + 43.8, period);
    if (localT < 1.7) {
      float prog = localT / 1.7;
      float ang = -0.38;
      float ca = cos(ang), sa = sin(ang);
      vec2 rotUv = vec2(uv.x * ca - uv.y * sa, uv.x * sa + uv.y * ca);
      
      float headX = mix(-1.8, 1.6, prog);
      float trackY = 0.52;
      float dx = rotUv.x - headX;
      float dy = abs(rotUv.y - trackY);
      float trailLen = 0.68;
      
      if (dx < 0.04 && dx > -trailLen && dy < 0.004) {
        float headCore = exp(-dx * dx * 2400.0) * exp(-dy * dy * 70000.0);
        float tailFade = pow(max(0.0, 1.0 + dx / trailLen), 2.2);
        float trailGlow = tailFade * exp(-dy * dy * 40000.0);
        float mIntensity = headCore * 2.0 + trailGlow * 1.15;
        
        vec3 colLight = mix(vec3(1.0, 0.46, 0.08), vec3(1.0, 0.98, 0.92), clamp(headCore * 1.4, 0.0, 1.0));
        vec3 colDark = mix(vec3(0.96, 0.42, 0.85), vec3(1.0, 1.0, 1.0), clamp(headCore * 1.4, 0.0, 1.0));
        vec3 mCol = mix(colLight, colDark, theme);
        total += vec4(mCol * mIntensity, clamp(mIntensity * 0.95, 0.0, 1.0));
      }
    }
  }
  
  return total;
}

void main() {
  float aspect = uResolution.x / max(uResolution.y, 1.0);
  vec2 screenUv = gl_FragCoord.xy / max(uResolution, vec2(1.0));
  vec2 sp = (screenUv - uSingularityCenter) * vec2(aspect, 1.0);
  float az = uMouse.x * 0.02;
  float el = 0.05 + uMouse.y * 0.015;
  vec3 ro = CAM_DIST * vec3(cos(el) * sin(az), sin(el), -cos(el) * cos(az));
  vec3 fwd = normalize(-ro);
  vec3 worldUp = vec3(0.0, 1.0, 0.0);
  vec3 right = normalize(cross(worldUp, fwd));
  vec3 up = cross(fwd, right);
  vec3 rd = normalize(fwd * FOCAL + right * sp.x + up * sp.y);

  vec3 pos = ro;
  float b = dot(ro, rd);
  float disc = b * b - (dot(ro, ro) - 529.0);
  if (disc > 0.0) {
    float tEnter = -b - sqrt(disc);
    if (tEnter > 0.0) pos = ro + rd * tEnter;
  }

  vec3 hv = cross(pos, rd);
  float h2 = dot(hv, hv);
  vec3 vel = rd;
  vec3 col = vec3(0.0);
  float through = 1.0;
  bool captured = false;

  for (int i = 0; i < STEPS; i++) {
    float r2 = dot(pos, pos);
    float r = sqrt(r2);

    if (r < 1.0) { 
      captured = true; 
      float rim = pow(clamp(r, 0.0, 1.0), 3.0);
      vec3 blackHoleCore = mix(vec3(0.003, 0.005, 0.008), vec3(0.15, 0.03, 0.01), rim);
      
      float whPulse = sin(uTime * 3.6) * 0.15 + 0.85;
      vec3 whiteHoleCore = mix(vec3(0.015, 0.02, 0.04), vec3(1.2, 1.4, 1.8), pow(rim, 4.0)) * whPulse;
      col += mix(blackHoleCore, whiteHoleCore, uTheme) * through;
      through = 0.0;
      break; 
    }

    if (r > 22.0 && dot(pos, vel) > 0.0) break;
    float dt = clamp(0.36 * (r - 0.95), 0.040, 0.96);
    vel += (-1.5 * h2 * pos / (r2 * r2 * r)) * dt;
    vec3 next = pos + vel * dt;

    if (pos.y * next.y < 0.0) {
      vec3 hit = mix(pos, next, pos.y / (pos.y - next.y));
      float hr = length(hit.xz);
      if (hr > DISK_IN && hr < DISK_OUT) {
        vec4 e = diskShade(hit, vel, uTheme);
        col += e.rgb * through;
        through *= (1.0 - e.a);
        if (through < 0.02) break;
      }
    }
    pos = next;
  }

  if (!captured) {
    vec3 bg = background(vel, uTheme);
    col += bg * through;
  }

  vec4 meteor = calcMeteors(sp, uTime, uTheme);
  if (meteor.a > 0.001) {
    vec3 meteorLight = mix(col, meteor.rgb, meteor.a * 0.92);
    vec3 meteorDark = col + meteor.rgb;
    col = mix(meteorLight, meteorDark, uTheme);
  }

  vec2 hp = sp * vec2(1.0, 1.75);
  float halo = exp(-length(hp) * 1.8) * 0.16;
  
  vec3 haloLight = vec3(1.0, 0.45, 0.08);
  vec3 haloDark = mix(uTintA, uTintB, 0.6);
  vec3 haloCol = mix(haloLight * 0.25, haloDark * 1.6, uTheme);
  col += haloCol * halo * smoothstep(0.06, 0.16, length(sp));

  // 连续无级色调映射：彻底消除 0.5 处的阶跃硬切与跳动
  vec3 colToneLight = clamp(col, 0.0, 1.0);
  
  vec3 colToneDark = col / (1.0 + col * 0.45);
  colToneDark = 1.0 - exp(-colToneDark * 1.8);
  colToneDark *= 0.95 + 0.10 * hash21(gl_FragCoord.xy * 0.73);

  col = mix(colToneLight, colToneDark, smoothstep(0.0, 1.0, uTheme));

  gl_FragColor = vec4(col, 1.0);
}`

export const GargantuaCanvas: React.FC<GargantuaCanvasProps> = ({ isDark, onFpsUpdate }) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const isDarkRef = useRef(isDark)
  isDarkRef.current = isDark

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    const gl =
      canvas.getContext('webgl2', {
        alpha: false,
        depth: false,
        antialias: false,
        powerPreference: 'high-performance',
      }) ||
      canvas.getContext('webgl', {
        alpha: false,
        depth: false,
        antialias: false,
        powerPreference: 'high-performance',
      })

    if (!gl) return

    const compile = (type: number, src: string) => {
      const s = gl.createShader(type)
      if (!s) return null
      gl.shaderSource(s, src)
      gl.compileShader(s)
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
        gl.deleteShader(s)
        return null
      }
      return s
    }

    const vs = compile(gl.VERTEX_SHADER, VS_SRC)
    const fs = compile(gl.FRAGMENT_SHADER, FS_SRC)
    if (!vs || !fs) {
      if (vs) gl.deleteShader(vs)
      if (fs) gl.deleteShader(fs)
      return
    }

    const prog = gl.createProgram()
    if (!prog) {
      gl.deleteShader(vs)
      gl.deleteShader(fs)
      return
    }
    gl.attachShader(prog, vs)
    gl.attachShader(prog, fs)
    gl.linkProgram(prog)

    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      gl.deleteProgram(prog)
      gl.deleteShader(vs)
      gl.deleteShader(fs)
      return
    }
    gl.useProgram(prog)

    const buf = gl.createBuffer()
    gl.bindBuffer(gl.ARRAY_BUFFER, buf)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW)
    const posLoc = gl.getAttribLocation(prog, 'position')
    gl.enableVertexAttribArray(posLoc)
    gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 0, 0)

    const uTimeLoc = gl.getUniformLocation(prog, 'uTime')
    const uResolutionLoc = gl.getUniformLocation(prog, 'uResolution')
    const uMouseLoc = gl.getUniformLocation(prog, 'uMouse')
    const uThemeLoc = gl.getUniformLocation(prog, 'uTheme')
    const uSingularityCenterLoc = gl.getUniformLocation(prog, 'uSingularityCenter')
    const uTintALoc = gl.getUniformLocation(prog, 'uTintA')
    const uTintBLoc = gl.getUniformLocation(prog, 'uTintB')
    const uTintCLoc = gl.getUniformLocation(prog, 'uTintC')

    gl.uniform3f(uTintALoc, 0.388, 0.4, 0.945)
    gl.uniform3f(uTintBLoc, 0.988, 0.451, 0.749)
    gl.uniform3f(uTintCLoc, 0.42, 0.298, 1.0)

    let curSingX = 0.35,
      curSingY = 0.5
    let targetSingX = 0.35,
      targetSingY = 0.5

    const updateSingularityCenterTarget = () => {
      const isDesktop = window.innerWidth >= 1024
      if (isDesktop) {
        const dock = document.getElementById('command-dock')
        const dockW = dock ? dock.offsetWidth : 440
        const viewW = Math.max(window.innerWidth - dockW, 100)
        targetSingX = (viewW * 0.5) / window.innerWidth
        targetSingY = 0.5
      } else {
        targetSingX = 0.5
        targetSingY = 0.72
      }
    }

    const resize = () => {
      // 动态分辨率自适应：将内部物理渲染分辨率限制在合理区间
      // 避免在 4K/Retina 显示器上盲目全量计算 600 万像素，利用 GPU 硬件双线性插值平滑呈现
      const dpr = window.devicePixelRatio || 1
      let scale = Math.min(dpr, 1.0)
      if (window.innerWidth > 1920) {
        scale = 0.82
      } else if (window.innerWidth <= 768) {
        scale = Math.min(dpr, 1.0)
      }
      const w = Math.round(window.innerWidth * scale)
      const h = Math.round(window.innerHeight * scale)
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w
        canvas.height = h
        gl.viewport(0, 0, w, h)
      }
      gl.uniform2f(uResolutionLoc, w, h)
      updateSingularityCenterTarget()
    }

    window.addEventListener('resize', resize)
    resize()

    let targetMx = 0,
      targetMy = 0
    let curMx = 0,
      curMy = 0

    const onPointerMove = (e: PointerEvent) => {
      const cx = window.innerWidth * 0.5
      const cy = window.innerHeight * 0.5
      targetMx = (e.clientX - cx) / cx
      targetMy = (e.clientY - cy) / cy
    }
    window.addEventListener('pointermove', onPointerMove, { passive: true })

    const startTime = performance.now()
    let currentTheme = isDarkRef.current ? 1.0 : 0.0
    let animId = 0
    let frames = 0
    let lastFpsTime = performance.now()

    const render = () => {
      const now = performance.now()
      const elapsed = (now - startTime) * 0.001

      curMx += (targetMx - curMx) * 0.04
      curMy += (targetMy - curMy) * 0.04

      const targetTheme = isDarkRef.current ? 1.0 : 0.0
      currentTheme += (targetTheme - currentTheme) * 0.075

      curSingX += (targetSingX - curSingX) * 0.05
      curSingY += (targetSingY - curSingY) * 0.05

      gl.uniform1f(uTimeLoc, elapsed)
      gl.uniform1f(uThemeLoc, currentTheme)
      gl.uniform2f(uMouseLoc, curMx, curMy)
      gl.uniform2f(uSingularityCenterLoc, curSingX, curSingY)
      gl.drawArrays(gl.TRIANGLES, 0, 3)

      frames++
      if (now - lastFpsTime >= 1000) {
        if (onFpsUpdate) {
          onFpsUpdate(Math.round((frames * 1000) / (now - lastFpsTime)))
        }
        frames = 0
        lastFpsTime = now
      }

      animId = requestAnimationFrame(render)
    }

    animId = requestAnimationFrame(render)

    // 智能休眠（Page Visibility API）：切入后台标签页时立即暂停 WebGL RAF，GPU 占用瞬间降至 0%
    const onVisibilityChange = () => {
      if (document.hidden) {
        if (animId) {
          cancelAnimationFrame(animId)
          animId = 0
        }
      } else if (!animId) {
        lastFpsTime = performance.now()
        frames = 0
        animId = requestAnimationFrame(render)
      }
    }
    document.addEventListener('visibilitychange', onVisibilityChange)

    // 处理 WebGL 上下文丢失防护（如系统休眠唤醒），防止上下文彻底报废
    const onContextLost = (e: Event) => {
      e.preventDefault()
      if (animId) {
        cancelAnimationFrame(animId)
        animId = 0
      }
    }
    canvas.addEventListener('webglcontextlost', onContextLost, false)

    return () => {
      if (animId) {
        cancelAnimationFrame(animId)
      }
      canvas.removeEventListener('webglcontextlost', onContextLost)
      document.removeEventListener('visibilitychange', onVisibilityChange)
      window.removeEventListener('resize', resize)
      window.removeEventListener('pointermove', onPointerMove)
      gl.deleteProgram(prog)
      gl.deleteShader(vs)
      gl.deleteShader(fs)
      gl.deleteBuffer(buf)
    }
  }, [onFpsUpdate])

  return (
    <canvas
      ref={canvasRef}
      id="gargantua-canvas"
      className="pointer-events-none fixed inset-0 z-0 opacity-100 transition-opacity duration-1000 ease-out"
    />
  )
}
