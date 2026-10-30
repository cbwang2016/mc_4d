//! Code-native teaching diagrams. The combinatorics and slicing use the actual
//! four-dimensional hypercube, not a decorative pair of nested cubes.
use crate::i18n::Language;
use egui::{Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, Vec2};
use mc4d_core::glam::{Vec3, Vec4};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diagram { Dimensions, SquareFaces, CubicCells, ProjectionSlice, RotatingSlice }

#[derive(Clone, Debug)]
pub struct Hypercube {
    pub vertices: Vec<Vec4>,
    pub edges: Vec<[usize; 2]>,
    pub faces: Vec<[usize; 4]>,
    pub cells: Vec<[usize; 8]>,
}

impl Hypercube {
    pub fn new(dimensions: usize) -> Self {
        assert!((2..=4).contains(&dimensions));
        let vertices = (0..1usize<<dimensions).map(|bits| {
            let mut v = Vec4::ZERO;
            for axis in 0..dimensions { v[axis] = if bits & (1<<axis) == 0 { -1.0 } else { 1.0 }; }
            v
        }).collect();
        let mut edges = Vec::new();
        let mut faces = Vec::new();
        let mut cells = Vec::new();
        for base in 0..1usize<<dimensions {
            for a in 0..dimensions {
                if base & (1<<a) != 0 { continue; }
                edges.push([base,base|(1<<a)]);
                for b in a+1..dimensions {
                    if base & (1<<b) != 0 { continue; }
                    faces.push([base,base|(1<<a),base|(1<<a)|(1<<b),base|(1<<b)]);
                    for c in b+1..dimensions {
                        if base & (1<<c) != 0 { continue; }
                        cells.push(std::array::from_fn(|bits| base
                            | ((bits&1)<<a) | (((bits>>1)&1)<<b) | (((bits>>2)&1)<<c)));
                    }
                }
            }
        }
        Self { vertices,edges,faces,cells }
    }
}

fn language<'a>(lang: Language, en: &'a str, zh: &'a str) -> &'a str {
    if lang == Language::Chinese { zh } else { en }
}

fn turn(mut v: Vec4, a: usize, b: usize, angle: f32) -> Vec4 {
    let (s,c) = angle.sin_cos();
    let (x,y) = (v[a],v[b]);
    v[a] = c*x-s*y;
    v[b] = s*x+c*y;
    v
}

fn rotate4(v: Vec4, time: f32) -> Vec4 {
    let v = turn(v,0,3,time*0.27);
    turn(v,2,3,0.35+(time*0.17).sin()*0.28)
}

/// Perspective projection of a 4D point into 3D; all diagram vertices stay clear
/// of its W=4 camera plane, including during combined rotations.
fn project4(v: Vec4) -> Vec3 { v.truncate() * (2.7/(4.0-v.w)) }

fn project3(mut v: Vec3, center: Pos2, scale: f32) -> Pos2 {
    let (s,c) = 0.54_f32.sin_cos();
    v = Vec3::new(c*v.x+s*v.z,v.y,-s*v.x+c*v.z);
    let (s,c) = (-0.35_f32).sin_cos();
    v = Vec3::new(v.x,c*v.y-s*v.z,s*v.y+c*v.z);
    let perspective = 5.0/(6.0-v.z);
    center+Vec2::new(v.x,-v.y)*scale*perspective
}

/// Every square face intersects a generic 3D hyperplane in an edge. Together
/// these segments are the wireframe of the true 3D cross-section.
pub fn slice_edges(cube: &Hypercube, rotated: &[Vec4], offset: f32) -> Vec<[Vec3;2]> {
    let mut segments = Vec::new();
    for face in &cube.faces {
        let mut points: Vec<Vec3> = Vec::new();
        for edge in 0..4 {
            let a = rotated[face[edge]];
            let b = rotated[face[(edge+1)%4]];
            let da = a.w-offset;
            let db = b.w-offset;
            if da.abs()<1e-5 && !points.iter().any(|p|p.distance_squared(a.truncate())<1e-8) { points.push(a.truncate()); }
            if da*db < -1e-10 {
                let point = a.lerp(b,da/(da-db)).truncate();
                if !points.iter().any(|p|p.distance_squared(point)<1e-8) { points.push(point); }
            }
        }
        if points.len()==2 && points[0].distance_squared(points[1])>1e-8 { segments.push([points[0],points[1]]); }
        else if points.len()>2 {
            // Coincident plane: the complete square is part of the slice boundary.
            for i in 0..points.len() { segments.push([points[i],points[(i+1)%points.len()]]); }
        }
    }
    segments
}

fn label(p: &egui::Painter, pos: Pos2, text: impl Into<String>, size: f32, color: Color32) {
    p.text(pos,Align2::CENTER_CENTER,text.into(),FontId::proportional(size),color);
}

fn wireframe(p: &egui::Painter, cube: &Hypercube, points: &[Pos2], face: Option<usize>, cell: Option<usize>) {
    let blue = Color32::from_rgb(95,205,255);
    let gold = Color32::from_rgb(255,202,95);
    if let Some(index) = cell {
        let vertices = &cube.cells[index];
        for square in &cube.faces {
            if square.iter().all(|v|vertices.contains(v)) {
                p.add(Shape::convex_polygon(square.iter().map(|&v|points[v]).collect(),Color32::from_rgba_unmultiplied(255,180,50,22),Stroke::NONE));
            }
        }
    }
    if let Some(index) = face {
        p.add(Shape::convex_polygon(cube.faces[index].iter().map(|&v|points[v]).collect(),Color32::from_rgba_unmultiplied(50,180,255,85),Stroke::new(2.5,blue)));
    }
    for edge in &cube.edges {
        let active = cell.is_some_and(|index|edge.iter().all(|v|cube.cells[index].contains(v)));
        p.line_segment([points[edge[0]],points[edge[1]]],Stroke::new(if active { 3.0 } else { 1.15 },
            if active { gold } else { Color32::from_rgba_unmultiplied(165,180,215,155) }));
    }
    for (index,&point) in points.iter().enumerate() {
        let active = cell.is_some_and(|c|cube.cells[c].contains(&index));
        p.circle_filled(point,if active { 3.5 } else { 2.3 },if active { gold } else { Color32::from_gray(215) });
    }
}

pub fn draw(ui: &mut egui::Ui, diagram: Diagram, time: f32, lang: Language, height: f32) {
    let (rect,_) = ui.allocate_exact_size(Vec2::new(ui.available_width(),height),Sense::hover());
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect,8.0,Color32::from_rgb(11,17,30));
    let white = Color32::from_rgb(225,232,245);
    let cyan = Color32::from_rgb(95,205,255);
    let gold = Color32::from_rgb(255,202,95);
    match diagram {
        Diagram::Dimensions => {
            for index in 0..3 {
                let dims = index+2;
                let cube = Hypercube::new(dims);
                let width = rect.width()/3.0;
                let center = Pos2::new(rect.left()+width*(index as f32+0.5),rect.center().y+2.0);
                let scale = ((height-80.0).max(30.0)/3.3).min(width*0.25);
                let progress = (((time+2.0)%10.0)/4.0).min(1.0);
                let points: Vec<_> = cube.vertices.iter().map(|&v| {
                    let mut v = v;
                    if dims>2 && v[dims-1]>0.0 { v[dims-1] = -1.0+2.0*progress; }
                    if dims==2 { center+Vec2::new(v.x,-v.y)*scale*0.75 }
                    else { project3(if dims==4 { project4(rotate4(v,time*0.3)) } else { v.truncate() },center,scale) }
                }).collect();
                wireframe(&p,&cube,&points,None,None);
                label(&p,Pos2::new(center.x,rect.top()+20.0),
                    language(lang,["Square · 2D","Cube · 3D","Tesseract · 4D"][index],["正方形 · 二维","立方体 · 三维","超立方体 · 四维"][index]),15.0,white);
                label(&p,Pos2::new(center.x,rect.bottom()-20.0),
                    language(lang,["4 edges","6 square faces","8 cubic cells"][index],["4 条边","6 个正方形面","8 个立方体胞"][index]),14.0,cyan);
                if index<2 { label(&p,Pos2::new(rect.left()+width*(index as f32+1.0),rect.center().y),"→",23.0,gold); }
            }
        }
        Diagram::SquareFaces|Diagram::CubicCells => {
            let cube = Hypercube::new(4);
            let scale = (height-80.0).max(30.0)/3.0;
            let points: Vec<_> = cube.vertices.iter().map(|&v|project3(project4(rotate4(v,time+0.8)),rect.center(),scale)).collect();
            let index = (time/2.5) as usize;
            let cells = diagram==Diagram::CubicCells;
            wireframe(&p,&cube,&points,(!cells).then_some(index%24),cells.then_some(index%8));
            let detail = if cells {
                if lang==Language::Chinese { format!("高亮立方体胞 {}/8：8 个顶点 · 12 条边 · 6 个正方形面",index%8+1) }
                else { format!("Cubic cell {}/8: 8 vertices · 12 edges · 6 square faces",index%8+1) }
            } else if lang==Language::Chinese { format!("高亮正方形面 {}/24 · 这个面同时属于两个立方体胞",index%24+1) }
            else { format!("Square face {}/24 · shared by two cubic cells",index%24+1) };
            label(&p,Pos2::new(rect.center().x,rect.bottom()-17.0),detail,14.0,if cells { gold } else { cyan });
            label(&p,Pos2::new(rect.center().x,rect.top()+17.0),language(lang,
                "16 vertices  ·  32 edges  ·  24 squares  ·  8 cubes",
                "16 个顶点  ·  32 条边  ·  24 个正方形面  ·  8 个立方体胞"),15.0,white);
        }
        Diagram::ProjectionSlice|Diagram::RotatingSlice => {
            let cube = Hypercube::new(4);
            let tilted = diagram==Diagram::RotatingSlice;
            let rotated: Vec<_> = cube.vertices.iter().map(|&v|if tilted {
                turn(turn(turn(v,0,3,0.5+(time*0.2).sin()*0.4),2,3,0.55),1,3,0.35)
            } else { v }).collect();
            let offset = if tilted { (time*0.35).sin()*1.6 } else { (time*0.5).sin()*1.3 };
            let left = Pos2::new(rect.left()+rect.width()*0.25,rect.center().y+4.0);
            let right = Pos2::new(rect.left()+rect.width()*0.75,rect.center().y+4.0);
            // Reserve room for headings even when an oblique section grows taller
            // than the axis-aligned cube during the animation.
            let scale = ((height-80.0).max(30.0)/3.6).min(rect.width()*0.13);
            let projection: Vec<_> = rotated.iter().map(|&v|project3(project4(v),left,scale)).collect();
            wireframe(&p,&cube,&projection,None,None);
            let edges = slice_edges(&cube,&rotated,offset);
            for edge in &edges {
                p.line_segment(edge.map(|v|project3(v,right,scale)),Stroke::new(2.3,cyan));
            }
            if edges.is_empty() {
                label(&p,right,language(lang,"No intersection","没有交集"),17.0,cyan);
            }
            label(&p,Pos2::new(left.x,rect.top()+20.0),language(lang,"Whole object: projection","完整物体的投影"),15.0,white);
            label(&p,Pos2::new(right.x,rect.top()+20.0),language(lang,"Actual 3D cross-section","实际的三维截面"),15.0,cyan);
            p.line_segment([Pos2::new(rect.center().x,rect.top()+40.0),Pos2::new(rect.center().x,rect.bottom()-35.0)],Stroke::new(1.0,Color32::from_gray(60)));
            label(&p,Pos2::new(rect.center().x,rect.bottom()-17.0),
                if lang==Language::Chinese { format!("自动演示 · 截面位置 W = {offset:+.2} · 左边的完整物体始终存在") }
                else { format!("Animated slice at W = {offset:+.2} · the complete object still exists") },14.0,gold);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tesseract_has_unique_combinatorial_features_and_shared_faces() {
        let h = Hypercube::new(4);
        assert_eq!((h.vertices.len(),h.edges.len(),h.faces.len(),h.cells.len()),(16,32,24,8));
        for square in &h.faces {
            assert_eq!(h.cells.iter().filter(|cell|square.iter().all(|v|cell.contains(v))).count(),2);
        }
        for cell in &h.cells {
            assert_eq!(h.edges.iter().filter(|edge|edge.iter().all(|v|cell.contains(v))).count(),12);
            assert_eq!(h.faces.iter().filter(|face|face.iter().all(|v|cell.contains(v))).count(),6);
        }
        let mut unique = std::collections::HashSet::new();
        for mut face in h.faces { face.sort(); assert!(unique.insert(face)); }
        let cube = Hypercube::new(3);
        assert_eq!((cube.vertices.len(),cube.edges.len(),cube.faces.len()),(8,12,6));
        let square = Hypercube::new(2);
        assert_eq!((square.vertices.len(),square.edges.len()),(4,4));
    }
    #[test]
    fn projection_stays_finite_through_rotations_and_slice_vanishes_outside() {
        let h = Hypercube::new(4);
        for tick in 0..500 {
            for &v in &h.vertices {
                let p = project3(project4(rotate4(v,tick as f32*0.05)),Pos2::ZERO,100.0);
                assert!(p.x.is_finite() && p.y.is_finite());
                assert!(p.x.abs()<150.0 && p.y.abs()<150.0,"diagram must stay inside its reserved drawing area");
            }
        }
        assert_eq!(slice_edges(&h,&h.vertices,0.0).len(),12);
        assert!(slice_edges(&h,&h.vertices,1.1).is_empty());
        let edges = slice_edges(&h,&h.vertices,0.0);
        for edge in edges { for v in edge { assert_eq!(v.abs(),Vec3::ONE); } }
    }
}
