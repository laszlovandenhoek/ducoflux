use std::time::{Duration, SystemTime, Instant};

use influxdb::{Client, Timestamp, WriteQuery};
use regex::Regex;
use serialport::SerialPort;
use tokio::time::{self, Duration as TDuration};

async fn write_to_serial(port: &mut dyn SerialPort, command: &str) {
    for byte in command.as_bytes() {
        port.write_all(&[*byte]).expect("Failed to write to serial port");
        time::sleep(Duration::from_millis(20)).await; // 0.02 second delay
    }
    port.write_all(b"\r").expect("Failed to write carriage return"); // Only use carriage return
}

fn read_from_serial(port: &mut dyn SerialPort, expected_size: usize) -> String {
    let mut buf = Vec::new();
    let mut temp_buf = vec![0; 1024];
    let mut total_bytes_read = 0;
    let timeout = Duration::from_secs(1);
    let start = Instant::now();

    while total_bytes_read < expected_size && start.elapsed() < timeout {
        match port.read(&mut temp_buf) {
            Ok(nbytes) => {
                total_bytes_read += nbytes;
                buf.extend_from_slice(&temp_buf[..nbytes]);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => (),
            Err(e) => panic!("Failed to read from serial port: {:?}", e),
        }
    }

    String::from_utf8_lossy(&buf).replace('\r', "\n").to_string()
}

fn extract_values(sensor_info: &str, fan_speed: &str) -> (Option<f64>, Option<f64>, Option<i32>, Option<i32>) {
    println!("sensorinfo: {sensor_info}");
    println!("fanspeed: {fan_speed}");

    let re_sensor = Regex::new(r"RH : (\d+) \[\.01%] \(0\)\n\s{2}TEMP :\s{2}(\d+)").unwrap();
    let re_fan = Regex::new(r"FanSpeed:\s*Actual\s*(\d+)\s*\[.*?]\s*-\s*Filtered\s*(\d+)\s*\[.*?]").unwrap();

    let humidity_and_temperature = re_sensor.captures(sensor_info).map(|cap| (
        cap[1].parse::<f64>().unwrap() / 100.0, //humidity
        cap[2].parse::<f64>().unwrap() / 10.0 // temperature
    ));

    let fan_speeds = re_fan.captures(fan_speed).map(|cap| (
        cap[1].parse::<i32>().unwrap(), //actual
        cap[2].parse::<i32>().unwrap() //filtered
    ));

    (
        humidity_and_temperature.map(|ht| ht.0),
        humidity_and_temperature.map(|ht| ht.1),
        fan_speeds.map(|fs| fs.0),
        fan_speeds.map(|fs| fs.1)
    )
}

async fn send_to_influxdb(humidity: Option<f64>, temperature: Option<f64>, fan_actual: Option<i32>, fan_filtered: Option<i32>) {
    let client = Client::new("http://192.168.1.218:8086", "ventilation");
    let now = Timestamp::Seconds(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() as u128);
    let query = WriteQuery::new(now, "ducozolder")
        .add_field("humidity", humidity)
        .add_field("temperature", temperature)
        .add_field("fan_actual", fan_actual)
        .add_field("fan_filtered", fan_filtered);

    if let Err(e) = client.query(&query).await {
        eprintln!("Failed to write to InfluxDB: {}", e);
    }

}

#[tokio::main]
async fn main() {
    let mut port = serialport::new("/dev/ttyUSB0", 112000)
        .timeout(Duration::from_secs(1))
        .open().expect("Failed to open port");

    let mut interval = time::interval(TDuration::from_secs(10));
    loop {
        interval.tick().await;
        // Write commands and read responses
        write_to_serial(port.as_mut(), "sensorinfo").await;
        let sensor_info = read_from_serial(port.as_mut(), 74);

        write_to_serial(port.as_mut(), "fanspeed").await;
        let fan_speed = read_from_serial(port.as_mut(), 65);

        // Extract and convert values
        let (
            humidity,
            temperature,
            fan_actual,
            fan_filtered
        ) = extract_values(&sensor_info, &fan_speed);

        // Send to InfluxDB
        send_to_influxdb(humidity, temperature, fan_actual, fan_filtered).await;
    }
}
