use types::{DetectionLineDirection, DetectionPoint};

/// 点在多边形内判定（射线法 Ray Casting Algorithm）
pub fn point_in_polygon(point: (f64, f64), polygon: &[DetectionPoint]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let (px, py) = point;
    let mut inside = false;
    let mut j = polygon.len() - 1;

    for i in 0..polygon.len() {
        let pi = &polygon[i];
        let pj = &polygon[j];

        if ((pi.y > py) != (pj.y > py)) && (px < (pj.x - pi.x) * (py - pi.y) / (pj.y - pi.y) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }

    inside
}

/// 检查目标移动轨迹 [p1 -> p2] 是否跨越绊线 [line_a -> line_b]
pub fn check_line_crossing(
    p1: (f64, f64),
    p2: (f64, f64),
    line_a: DetectionPoint,
    line_b: DetectionPoint,
    direction: DetectionLineDirection,
) -> bool {
    let a = (line_a.x, line_a.y);
    let b = (line_b.x, line_b.y);

    // 快速排斥与跨立实验判定线段相交
    if !segments_intersect(p1, p2, a, b) {
        return false;
    }

    match direction {
        DetectionLineDirection::Both => true,
        DetectionLineDirection::AToB => {
            // 计算向量 (b - a) 与 (p2 - p1) 的叉积方向
            let cross = cross_product((b.0 - a.0, b.1 - a.1), (p2.0 - p1.0, p2.1 - p1.1));
            cross > 0.0
        }
        DetectionLineDirection::BToA => {
            let cross = cross_product((b.0 - a.0, b.1 - a.1), (p2.0 - p1.0, p2.1 - p1.1));
            cross < 0.0
        }
    }
}

fn cross_product(v1: (f64, f64), v2: (f64, f64)) -> f64 {
    v1.0 * v2.1 - v1.1 * v2.0
}

fn ccw(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> bool {
    (c.1 - a.1) * (b.0 - a.0) > (b.1 - a.1) * (c.0 - a.0)
}

fn segments_intersect(p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), p4: (f64, f64)) -> bool {
    ccw(p1, p3, p4) != ccw(p2, p3, p4) && ccw(p1, p2, p3) != ccw(p1, p2, p4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_in_polygon() {
        let polygon = vec![
            DetectionPoint::new(0.0, 0.0),
            DetectionPoint::new(1.0, 0.0),
            DetectionPoint::new(1.0, 1.0),
            DetectionPoint::new(0.0, 1.0),
        ];

        assert!(point_in_polygon((0.5, 0.5), &polygon));
        assert!(!point_in_polygon((1.5, 0.5), &polygon));
    }

    #[test]
    fn test_line_crossing() {
        let line_a = DetectionPoint::new(0.0, 0.5);
        let line_b = DetectionPoint::new(1.0, 0.5);

        // 从下向上穿过
        let p1 = (0.5, 0.2);
        let p2 = (0.5, 0.8);

        assert!(check_line_crossing(
            p1,
            p2,
            line_a,
            line_b,
            DetectionLineDirection::Both
        ));
    }
}
