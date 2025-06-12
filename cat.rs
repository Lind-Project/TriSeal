use std::env;
use std::fs::File;
use std::io::{self, BufReader, Read};

fn main() -> io::Result<()> {
    // Collect command-line arguments, skipping the program name.
//    let args: Vec<String> = env::args().skip(1).collect();

    println!("fucking irritating\n");
    let file = File::open("test.txt")?;
    //println!("fucking irritating part 2\n");
    // If no files are provided, read from stdin.
    // if args.is_empty() {
    //     let mut buffer = String::new();
    //     io::stdin().read_to_string(&mut buffer)?;
    //     print!("{}", buffer);
    // } else {
        // Otherwise, open each file and print its contents.
        // for filename in args {
        //     // println!("{}", filename);
        //     let file = File::open(&filename)?;
        //     //panic!("{:?}", file);
        //     println!("{:?}", file);
        //     let mut reader = BufReader::new(file);
        //     println!("{:?}", reader);
        //     let mut contents = String::new();
        //     reader.read_to_string(&mut contents)?;
        //     println!("{:?}", contents);
        //     print!("{}", contents);
        // }
    //}
    Ok(())
}
