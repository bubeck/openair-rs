//! Simple line-based parser for Skytraxx airspace files.

use std::fmt;
use std::io::BufRead;

use log::trace;

/// Airspace class.
#[derive(Debug, PartialEq, Eq)]
pub enum Class {
    /// Airspace C
    C,
    /// Airspace D
    D,
    /// Airspace E
    E,
    /// Airspace G
    G,
    /// Controlled Traffic Region
    CTR,
    /// Danger area (LS-D)
    Danger,
    /// Restricted area (LS-R)
    Restricted,
    /// Prohibited area (LS-P)
    Prohibited,
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Class {
    fn parse(data: &str) -> Result<Self, String> {
        match data {
            "C" => Ok(Class::C),
            "D" => Ok(Class::D),
            "E" => Ok(Class::E),
            "G" => Ok(Class::G),
            "CTR" => Ok(Class::CTR),
            "Q" => Ok(Class::Danger),
            "R" => Ok(Class::Restricted),
            "P" => Ok(Class::Prohibited),
            other => Err(format!("Invalid class: {}", other))
        }
    }
}

/// Altitude, either ground or a certain height AMSL in feet.
#[derive(Debug, PartialEq, Eq)]
pub enum Altitude {
    /// Ground level
    Gnd,
    /// Feet above mean sea level
    FeetAmsl(i32),
    /// Feet above ground level
    FeetAgl(i32),
}

impl fmt::Display for Altitude {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Altitude::Gnd => write!(f, "GND"),
            Altitude::FeetAmsl(ft) => write!(f, "{} ft AMSL", ft),
            Altitude::FeetAgl(ft) => write!(f, "{} ft AGL", ft),
        }
    }
}

impl Altitude {
    fn parse(data: &str) -> Result<Self, String> {
        if data == "GND" {
            Ok(Altitude::Gnd)
        } else {
            let is_digit = |c: &char| c.is_digit(10);
            let number: String = data.chars().take_while(is_digit).collect();
            let rest: String = data.chars().skip_while(is_digit).collect();
            match (number.parse::<i32>().ok(), &*rest) {
                (Some(ft), " ft") => Ok(Altitude::FeetAmsl(ft)),
                (Some(ft), " ft AGL") => Ok(Altitude::FeetAgl(ft)),
                _ => Err(format!("Invalid altitude: {:?}", data))
            }
        }
    }
}

/// A coordinate pair (WGS84).
#[derive(Debug, PartialEq)]
pub struct Coord {
    lat: f64,
    lng: f64,
}

impl Coord {
    fn parse_number_opt(val: Option<&str>) -> Result<u16, ()> {
        val.and_then(|v| v.parse::<u16>().ok()).ok_or(())
    }

    fn parse_component(val: &str) -> Result<f64, ()> {
        let mut parts = val.split(":");
        let deg = Self::parse_number_opt(parts.next())?;
        let min = Self::parse_number_opt(parts.next())?;
        let sec = Self::parse_number_opt(parts.next())?;
        Ok(deg as f64 + min as f64 / 60.0 + sec as f64 / 3600.0)
    }

    fn multiplier_lat(val: &str) -> Result<f64, ()> {
        match val {
            "N" => Ok(1.0),
            "S" => Ok(-1.0),
            _ => Err(())
        }
    }

    fn multiplier_lng(val: &str) -> Result<f64, ()> {
        match val {
            "E" => Ok(1.0),
            "W" => Ok(-1.0),
            _ => Err(())
        }
    }

    fn parse(data: &str) -> Result<Self, String> {
        let parts: Vec<&str> = data.split(" ").collect();
        let invalid = |_| format!("Invalid coord: {}", data);
        if parts.len() != 4 {
            return Err(invalid(()));
        }
        let lat = Self::multiplier_lat(parts[1]).map_err(invalid)?
                * Self::parse_component(parts[0]).map_err(invalid)?;
        let lng = Self::multiplier_lng(parts[3]).map_err(invalid)?
                * Self::parse_component(parts[2]).map_err(invalid)?;
        Ok(Coord { lat, lng })
    }
}

#[derive(Debug, PartialEq)]
enum Geometry {
    Polygon {
        points: Vec<Coord>
    },
    Circle {
        centerpoint: Coord,
        radius: f32,
    },
}

impl fmt::Display for Geometry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Geometry::Polygon { points } => write!(f, "Polygon[{}]", points.len()),
            Geometry::Circle { centerpoint: _, radius } => write!(f, "Circle[r={}km]", radius),
        }
    }
}

/// An airspace.
#[derive(Debug)]
pub struct Airspace {
    name: String,
    class: Class,
    lower_bound: Altitude,
    upper_bound: Altitude,
    geom: Geometry,
}

impl fmt::Display for Airspace {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} [{}] ({} → {}) {{{}}}",
            self.name,
            self.class,
            self.lower_bound,
            self.upper_bound,
            self.geom,
        )
    }
}

#[derive(Debug)]
enum ParsingState {
    New,
    HasClass(Class),
    HasName(Class, String),
    HasLowerBound(Class, String, Altitude),
    HasUpperBound(Class, String, Altitude, Altitude),
    ParsingPolygon(Class, String, Altitude, Altitude, Vec<Coord>),
    ParsingCircle(Class, String, Altitude, Altitude, Coord),
    Done(Airspace),
}

impl fmt::Display for ParsingState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match self {
            ParsingState::New => "New",
            ParsingState::HasClass(..) => "HasClass",
            ParsingState::HasName(..) => "HasName",
            ParsingState::HasLowerBound(..) => "HasLowerBound",
            ParsingState::HasUpperBound(..) => "HasUpperBound",
            ParsingState::ParsingPolygon(..) => "ParsingPolygon",
            ParsingState::ParsingCircle(..) => "ParsingCircle",
            ParsingState::Done(..) => "Done",
        })
    }
}

/// Process a line based on the current state. Return a new state or an error.
fn process(state: ParsingState, line: &str) -> Result<ParsingState, String> {
    let mut chars = line.chars();
    let t1 = chars.next().ok_or_else(|| "Line too short".to_string())?;
    let t2 = chars.next().ok_or_else(|| "Line too short".to_string())?;
    let data = line.get(3..).unwrap_or("").trim();
    trace!("State: \"{}\", Input: \"{:1}{:1}\"", state, t1, t2);
    match (state, t1, t2) {
        (ParsingState::New, '*', _) => {
            // Comment, ignore
            trace!("-> Ignore");
            Ok(ParsingState::New)
        }
        (ParsingState::New, 'A', 'C') => {
            // Airspace class
            trace!("-> Found class");
            let class = Class::parse(data)?;
            Ok(ParsingState::HasClass(class))
        }
        (ParsingState::HasClass(c), 'A', 'N') => {
            trace!("-> Found name");
            Ok(ParsingState::HasName(c, data.to_string()))
        }
        (ParsingState::HasName(c, n), 'A', 'L') => {
            trace!("-> Found lower bound");
            let lower = Altitude::parse(data)?;
            Ok(ParsingState::HasLowerBound(c, n, lower))
        }
        (ParsingState::HasLowerBound(c, n, l), 'A', 'H') => {
            trace!("-> Found upper bound");
            let upper = Altitude::parse(data)?;
            Ok(ParsingState::HasUpperBound(c, n, l, upper))
        }
        (ParsingState::HasUpperBound(c, n, l, u), 'D', 'P') => {
            trace!("-> Found point");
            let coords = vec![Coord::parse(data)?];
            Ok(ParsingState::ParsingPolygon(c, n, l, u, coords))
        }
        (ParsingState::HasUpperBound(c, n, l, u), 'V', _) => {
            trace!("-> Found centerpoint");
            let centerpoint = Coord::parse(data.get(1..).unwrap_or(""))?;
            Ok(ParsingState::ParsingCircle(c, n, l, u, centerpoint))
        }
        (ParsingState::ParsingPolygon(c, n, l, u, mut p), 'D', 'P') => {
            trace!("-> Found point");
            p.push(Coord::parse(data)?);
            Ok(ParsingState::ParsingPolygon(c, n, l, u, p))
        }
        (ParsingState::ParsingPolygon(c, n, l, u, p), '*', _) => {
            trace!("-> Done parsing polygon");
            if p.len() < 2 {
                return Err(format!("Invalid airspace polygon (only {} points)", p.len()));
            }
            Ok(ParsingState::Done(Airspace {
                name: n,
                class: c,
                lower_bound: l,
                upper_bound: u,
                geom: Geometry::Polygon { points: p },
            }))
        }
        (ParsingState::ParsingCircle(c, n, l, u, p), 'D', 'C') => {
            trace!("-> Found point");
            let radius = data.parse::<f32>().map_err(|_| format!("Invalid radius: {}", data))?;
            Ok(ParsingState::Done(Airspace {
                name: n,
                class: c,
                lower_bound: l,
                upper_bound: u,
                geom: Geometry::Circle { centerpoint: p, radius },
            }))
        }
        (state, t1, t2) => {
            Err(format!("Parse error in state \"{}\" (unexpected \"{:1}{:1}\")", state, t1, t2))
        }
    }
}

/// Process the reader line by line. Once an airspace has been found
/// completely, return that airspace. When the end of the reader has been
/// reached, return `None`.
pub fn parse<R: BufRead>(reader: &mut R) -> Result<Option<Airspace>, String> {
    let mut state = ParsingState::New;
    loop {
        // Read next line
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)
            .map_err(|e| format!("Could not read line: {}", e))?;
        if bytes_read == 0 {
            // EOF
            return Ok(None);
        }

        // Trim BOM
        let trimmed_line = line.trim_start_matches('\u{feff}');

        // Find next state
        state = process(state, trimmed_line)?;

        if let ParsingState::Done(airspace) = state {
            return Ok(Some(airspace))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_coord() {
        assert_eq!(
            Coord::parse("46:51:44 N 009:19:42 E"),
            Ok(Coord { lat: 46.86222222222222, lng: 9.328333333333333 })
        );
        assert_eq!(
            Coord::parse("46:51:44 S 009:19:42 W"),
            Ok(Coord { lat: -46.86222222222222, lng: -9.328333333333333 })
        );
        assert_eq!(
            Coord::parse("46:51:44 Q 009:19:42 R"),
            Err("Invalid coordinate: 46:51:44 Q 009:19:42 R".to_string())
        );
    }
}