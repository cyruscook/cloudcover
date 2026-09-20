package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"io"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"
)

type apiMethod struct {
	Service string `json:"service"`
	Name    string `json:"name"`
}
type mappingRow struct {
	Package    string      `json:"package"`
	Receiver   string      `json:"receiver"`
	Method     string      `json:"method"`
	APIMethods []apiMethod `json:"api_methods"`
}
type release struct {
	ModuleVersion string      `json:"module_version"`
	Remove        [][3]string `json:"remove"`
	Upsert        [][]any     `json:"upsert"`
}
type dataFile struct {
	ModulePath string    `json:"module_path"`
	Releases   []release `json:"releases"`
}
type sdkState map[string]mappingRow

type version struct {
	raw                 string
	major, minor, patch int
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}
func run() error {
	repository := flag.String("repository", "", "bare aws-sdk-go git repository")
	output := flag.String("output", "", "output JSON file")
	flag.Parse()
	if *repository == "" || *output == "" {
		return errors.New("--repository and --output are required")
	}
	tags, err := gitLines(*repository, "tag", "--list", "v1.*")
	if err != nil {
		return err
	}
	versions := make([]version, 0, len(tags))
	for _, tag := range tags {
		if parsed, ok := parseVersion(tag); ok {
			versions = append(versions, parsed)
		}
	}
	sort.Slice(versions, func(i, j int) bool { return lessVersion(versions[i], versions[j]) })
	if len(versions) == 0 {
		return errors.New("no stable v1 tags found")
	}
	state := sdkState{}
	batch, err := newGitBatch(*repository)
	if err != nil {
		return err
	}
	defer batch.close()
	var previous string
	result := dataFile{ModulePath: "github.com/aws/aws-sdk-go", Releases: make([]release, 0, len(versions))}
	for index, current := range versions {
		changed, err := changedServices(*repository, previous, current.raw, index == 0)
		if err != nil {
			return err
		}
		before := cloneState(state)
		for _, service := range changed {
			if err := updateService(batch, current.raw, service, state); err != nil {
				return err
			}
		}
		result.Releases = append(result.Releases, delta(before, state, current.raw))
		previous = current.raw
	}
	encoded, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}
	encoded = append(encoded, '\n')
	return os.WriteFile(*output, encoded, 0o644)
}
func delta(previous sdkState, current sdkState, raw string) release {
	result := release{
		ModuleVersion: strings.TrimPrefix(raw, "v"),
		Remove:        make([][3]string, 0),
		Upsert:        make([][]any, 0),
	}
	for key := range previous {
		if _, ok := current[key]; !ok {
			result.Remove = append(result.Remove, splitKey(key))
		}
	}
	for key, row := range current {
		old, ok := previous[key]
		if ok && equalRow(old, row) {
			continue
		}
		result.Upsert = append(result.Upsert, rowJSON(row))
	}
	sort.Slice(result.Remove, func(i, j int) bool { return fmt.Sprint(result.Remove[i]) < fmt.Sprint(result.Remove[j]) })
	sort.Slice(result.Upsert, func(i, j int) bool { return fmt.Sprint(result.Upsert[i]) < fmt.Sprint(result.Upsert[j]) })
	return result
}

func changedServices(repository, previous, current string, first bool) ([]string, error) {
	var lines []string
	var err error
	if first {
		lines, err = gitLines(repository, "ls-tree", "-r", "--name-only", current, "service")
	} else {
		lines, err = gitLines(repository, "diff", "--name-only", previous, current, "--", "service")
	}
	if err != nil {
		return nil, err
	}
	set := map[string]bool{}
	for _, line := range lines {
		parts := strings.Split(line, "/")
		if len(parts) >= 3 && parts[0] == "service" && parts[2] == "api.go" {
			set[parts[1]] = true
		}
	}
	result := make([]string, 0, len(set))
	for service := range set {
		result = append(result, service)
	}
	sort.Strings(result)
	return result, nil
}

func updateService(batch *gitBatch, tag, service string, state sdkState) error {
	prefix := "github.com/aws/aws-sdk-go/service/" + service + "/"
	for key := range state {
		if strings.HasPrefix(key, prefix) {
			delete(state, key)
		}
	}
	content, err := batch.show(tag, "service/"+service+"/api.go")
	if err != nil {
		return nil
	}
	rows, err := parseService(content, service)
	if err != nil {
		return fmt.Errorf("parse %s at %s: %w", service, tag, err)
	}
	for _, row := range rows {
		state[rowKey(row)] = row
	}
	return nil
}

func parseService(content []byte, service string) ([]mappingRow, error) {
	file, err := parser.ParseFile(token.NewFileSet(), "api.go", content, 0)
	if err != nil {
		return nil, err
	}
	type method struct{ receiver, name string }
	methods := map[string]map[string]bool{}
	for _, decl := range file.Decls {
		fn, ok := decl.(*ast.FuncDecl)
		if !ok || fn.Recv == nil || fn.Name == nil || !ast.IsExported(fn.Name.Name) {
			continue
		}
		if len(fn.Recv.List) != 1 {
			continue
		}
		receiver := receiverName(fn.Recv.List[0].Type)
		if receiver == "" {
			continue
		}
		if methods[receiver] == nil {
			methods[receiver] = map[string]bool{}
		}
		methods[receiver][fn.Name.Name] = true
	}
	rows := []mappingRow{}
	for receiver, names := range methods {
		for name := range names {
			base, ok := baseOperation(name)
			if !ok || !names[base+"Request"] {
				continue
			}
			rows = append(rows, mappingRow{Package: "github.com/aws/aws-sdk-go/service/" + service, Receiver: receiver, Method: name, APIMethods: []apiMethod{{Service: service, Name: base}}})
		}
	}
	sort.Slice(rows, func(i, j int) bool { return rowKey(rows[i]) < rowKey(rows[j]) })
	return rows, nil
}
func receiverName(expr ast.Expr) string {
	switch expr := expr.(type) {
	case *ast.StarExpr:
		return receiverName(expr.X)
	case *ast.Ident:
		return expr.Name
	}
	return ""
}
func baseOperation(name string) (string, bool) {
	for _, prefix := range []string{"WaitUntil", "Presign"} {
		if strings.HasPrefix(name, prefix) {
			return "", false
		}
	}
	for _, suffix := range []string{"PagesWithContext", "WithContext", "Pages"} {
		if strings.HasSuffix(name, suffix) {
			base := strings.TrimSuffix(name, suffix)
			return base, base != ""
		}
	}
	return name, name != ""
}
func rowKey(row mappingRow) string   { return row.Package + "\x00" + row.Receiver + "\x00" + row.Method }
func keyString(key [3]string) string { return key[0] + "\x00" + key[1] + "\x00" + key[2] }
func splitKey(key string) [3]string {
	parts := strings.SplitN(key, "\x00", 3)
	return [3]string{parts[0], parts[1], parts[2]}
}
func equalRow(a, b mappingRow) bool {
	return rowKey(a) == rowKey(b) && a.APIMethods[0] == b.APIMethods[0]
}
func rowJSON(row mappingRow) []any {
	methods := make([][]string, len(row.APIMethods))
	for i, method := range row.APIMethods {
		methods[i] = []string{method.Service, method.Name}
	}
	return []any{row.Package, row.Receiver, row.Method, methods}
}
func cloneState(state sdkState) sdkState {
	clone := sdkState{}
	for k, v := range state {
		clone[k] = v
	}
	return clone
}
func parseVersion(tag string) (version, bool) {
	if !strings.HasPrefix(tag, "v1.") {
		return version{}, false
	}
	parts := strings.Split(strings.TrimPrefix(tag, "v"), ".")
	if len(parts) != 3 {
		return version{}, false
	}
	major, e1 := strconv.Atoi(parts[0])
	minor, e2 := strconv.Atoi(parts[1])
	patch, e3 := strconv.Atoi(parts[2])
	return version{tag, major, minor, patch}, e1 == nil && e2 == nil && e3 == nil &&
		fmt.Sprintf("%d.%d.%d", major, minor, patch) == strings.TrimPrefix(tag, "v")
}
func lessVersion(a, b version) bool {
	if a.major != b.major {
		return a.major < b.major
	}
	if a.minor != b.minor {
		return a.minor < b.minor
	}
	return a.patch < b.patch
}
func gitLines(repository string, args ...string) ([]string, error) {
	output, err := runGit(repository, args...)
	if err != nil {
		return nil, err
	}
	lines := strings.Split(strings.TrimSpace(string(output)), "\n")
	if len(lines) == 1 && lines[0] == "" {
		return nil, nil
	}
	return lines, nil
}
func gitShow(repository, tag, file string) ([]byte, error) {
	return runGit(repository, "show", tag+":"+file)
}
func runGit(repository string, args ...string) ([]byte, error) {
	cmd := exec.Command("git", append([]string{"--git-dir", repository}, args...)...)
	output, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("git %v: %w", args, err)
	}
	return bytes.TrimSpace(output), nil
}

type gitBatch struct {
	input  io.WriteCloser
	output *bufio.Reader
	cmd    *exec.Cmd
}

func newGitBatch(repository string) (*gitBatch, error) {
	cmd := exec.Command("git", "--git-dir", repository, "cat-file", "--batch")
	input, err := cmd.StdinPipe()
	if err != nil {
		return nil, err
	}
	output, err := cmd.StdoutPipe()
	if err != nil {
		return nil, err
	}
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	return &gitBatch{input: input, output: bufio.NewReader(output), cmd: cmd}, nil
}

func (batch *gitBatch) show(tag, file string) ([]byte, error) {
	if _, err := fmt.Fprintf(batch.input, "%s:%s\n", tag, file); err != nil {
		return nil, err
	}
	header, err := batch.output.ReadString('\n')
	if err != nil {
		return nil, err
	}
	fields := strings.Fields(header)
	if len(fields) != 3 || fields[1] != "blob" {
		return nil, fmt.Errorf("git object %s:%s is unavailable", tag, file)
	}
	size, err := strconv.Atoi(fields[2])
	if err != nil {
		return nil, err
	}
	content := make([]byte, size)
	if _, err := io.ReadFull(batch.output, content); err != nil {
		return nil, err
	}
	if _, err := batch.output.ReadByte(); err != nil {
		return nil, err
	}
	return content, nil
}

func (batch *gitBatch) close() {
	_ = batch.input.Close()
	_ = batch.cmd.Wait()
}
