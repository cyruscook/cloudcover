package main

import (
  "fmt"
  "go/ast"
  "go/types"
  "path/filepath"
  "golang.org/x/tools/go/packages"
)

func main() {
  cfg := &packages.Config{Mode: packages.LoadAllSyntax, Dir: "../../../../target/cloudcover-build-cache/cloudcover-terraform-provider-aws/terraform-provider-aws", Tests: false}
  pkgs, err := packages.Load(cfg, "./internal/service/s3")
  if err != nil { panic(err) }
  if packages.PrintErrors(pkgs) > 0 { panic("load errors") }
  pkg := pkgs[0]
  for i, file := range pkg.Syntax {
    if filepath.Base(pkg.GoFiles[i]) != "bucket.go" { continue }
    ast.Inspect(file, func(node ast.Node) bool {
      fd, ok := node.(*ast.FuncDecl)
      if !ok || fd.Name.Name != "resourceBucketCreate" { return true }
      ast.Inspect(fd, func(node ast.Node) bool {
        call, ok := node.(*ast.CallExpr)
        if !ok { return true }
        sel, ok := call.Fun.(*ast.SelectorExpr)
        if !ok || sel.Sel.Name != "CreateBucket" { return true }
        pos := pkg.Fset.Position(sel.Pos())
        fmt.Println("POS", pos)
        if obj, ok := pkg.TypesInfo.ObjectOf(sel.Sel).(*types.Func); ok && obj != nil {
          fmt.Println("OBJECTOF", obj.Pkg().Path(), obj.Name(), recv(obj.Type()))
        } else {
          fmt.Println("OBJECTOF nil")
        }
        if seln, ok := pkg.TypesInfo.Selections[sel]; ok && seln != nil {
          if obj, ok := seln.Obj().(*types.Func); ok && obj != nil {
            fmt.Println("SELECTION", obj.Pkg().Path(), obj.Name(), recv(obj.Type()))
          } else {
            fmt.Println("SELECTION no func")
          }
        } else {
          fmt.Println("SELECTION nil")
        }
        return true
      })
      return false
    })
  }
}

func recv(t types.Type) string {
  sig, _ := t.(*types.Signature)
  if sig == nil || sig.Recv() == nil { return "" }
  rt := sig.Recv().Type()
  for {
    p, ok := rt.(*types.Pointer)
    if !ok { break }
    rt = p.Elem()
  }
  if n, ok := rt.(*types.Named); ok && n.Obj() != nil { return n.Obj().Name() }
  return fmt.Sprintf("%T", rt)
}
