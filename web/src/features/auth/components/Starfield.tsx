import React from 'react'

interface StarStyle extends React.CSSProperties {
  '--star-x': string
  '--star-y': string
  '--star-size': string
  '--star-glow-size': string
  '--star-low-opacity': string
  '--star-high-opacity': string
  '--star-duration': string
  '--star-delay': string
}

interface StarDefinition {
  x: number
  y: number
  size: number
  glowSize: number
  lowOpacity: number
  highOpacity: number
  duration: number
  delay: number
}

const STAR_COUNT = 72

function createStars(): readonly StarDefinition[] {
  let seed = 0x4d595df4
  const next = () => {
    seed = (seed * 1664525 + 1013904223) >>> 0
    return seed / 4294967296
  }

  return Array.from({ length: STAR_COUNT }, () => {
    const size = 0.8 + next() * 1.7
    const isBright = next() > 0.84

    return {
      x: next() * 100,
      y: next() * 100,
      size,
      glowSize: 1.5 + size * 2.2,
      lowOpacity: isBright ? 0.38 : 0.18,
      highOpacity: isBright ? 0.98 : 0.72,
      duration: 3.8 + next() * 5.2,
      delay: -(next() * 8.5),
    }
  })
}

const STARS = createStars()

export function Starfield(): React.ReactElement {
  return (
    <div aria-hidden="true" className="auth-starfield">
      {STARS.map((star, index) => {
        const style: StarStyle = {
          '--star-x': `${star.x.toFixed(2)}%`,
          '--star-y': `${star.y.toFixed(2)}%`,
          '--star-size': `${star.size.toFixed(2)}px`,
          '--star-glow-size': `${star.glowSize.toFixed(2)}px`,
          '--star-low-opacity': star.lowOpacity.toFixed(2),
          '--star-high-opacity': star.highOpacity.toFixed(2),
          '--star-duration': `${star.duration.toFixed(2)}s`,
          '--star-delay': `${star.delay.toFixed(2)}s`,
        }

        return <span key={`star-${index}`} className="auth-starfield__star" style={style} />
      })}
    </div>
  )
}
