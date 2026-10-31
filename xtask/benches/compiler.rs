use divan::Bencher;
use orisu_allocator::Allocator;
use orisu_canonical::{CanonicalizeOptions, Canonicalizer};
use orisu_compat_tailwind::Tailwind;
use orisu_compiler::compiler::{Compiler, Options, compile};
use orisu_compiler::intellisense::class_list;

const INPUT: &str = "@import \"tailwindcss\";";

fn main() {
    divan::main();
}

fn compiler(alloc: &Allocator) -> Compiler<'_> {
    compile(INPUT, alloc, Options { extension: Some(&Tailwind), ..Options::default() }).expect("the input compiles")
}

fn classes() -> Vec<String> {
    let alloc = Allocator::new();
    class_list(&compiler(&alloc).design_system).into_iter().map(|entry| entry.name).collect()
}

#[divan::bench]
fn compile_tailwind() {
    let alloc = Allocator::new();
    divan::black_box(compiler(&alloc));
}

#[divan::bench]
fn build_every_class(bencher: Bencher) {
    let classes = classes();
    bencher.bench_local(|| {
        let alloc = Allocator::new();
        divan::black_box(compiler(&alloc).build(classes.iter().map(String::as_str)).len());
    });
}

#[divan::bench]
fn canonicalize_every_class(bencher: Bencher) {
    let classes = classes();
    let classes: Vec<&str> = classes.iter().map(String::as_str).collect();
    let alloc = Allocator::new();
    let compiler = compiler(&alloc);
    bencher.bench_local(|| {
        let mut canonicalizer = Canonicalizer::new(&compiler.design_system, &alloc);
        divan::black_box(canonicalizer.canonicalize(&classes, &CanonicalizeOptions::default()));
    });
}
